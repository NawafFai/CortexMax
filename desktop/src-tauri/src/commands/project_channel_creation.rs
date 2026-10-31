use nostr::JsonUtil;
use tauri::{AppHandle, Manager, State};

use super::channel_workspaces::{require_idle_runtimes, scope};
use crate::{
    app_state::AppState, archive::project_channel_creations as journal, models::ChannelInfo,
};

/// Create an empty project folder without replacing files or following symlinks.
#[tauri::command]
pub async fn create_channel_project_folder(parent: String, name: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        crate::archive::workspace_folders::create_folder(&parent, &name)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Recover the original local creation request; never infer success from an empty relay response.
#[tauri::command]
pub async fn get_pending_project_channel(
    state: State<'_, AppState>,
    expected_owner: String,
    expected_relay: String,
) -> Result<Option<journal::ProjectChannelRequest>, String> {
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let pending = state
        .archive_db
        .with_conn(move |conn| journal::pending(conn, &owner, &relay))
        .await?;
    scope(&state, &expected_relay, &expected_owner)?;
    Ok(pending)
}

/// Persist the binding and exact signed create event together before any relay side effect.
#[tauri::command]
pub async fn create_project_channel(
    app: AppHandle,
    state: State<'_, AppState>,
    input: journal::ProjectChannelRequest,
    expected_owner: String,
    expected_relay: String,
) -> Result<ChannelInfo, String> {
    let _workspace = state.workspace_apply_lock.lock().await;
    let changes = std::sync::Arc::clone(&state.channel_workspace_change_lock)
        .write_owned()
        .await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    if input.request_id.is_nil() {
        return Err("creation request id must not be nil".into());
    }
    if !matches!(input.visibility.as_str(), "open" | "private")
        || !matches!(input.channel_type.as_str(), "stream" | "forum")
    {
        return Err("invalid channel type or visibility".into());
    }
    if input.name.len() > 256
        || input
            .description
            .as_ref()
            .is_some_and(|value| value.len() > 8192)
    {
        return Err("channel name or description exceeds the local creation limit".into());
    }
    let keys = state.signing_keys()?;
    let event = crate::events::build_create_channel(
        input.request_id,
        &input.name,
        &input.visibility,
        &input.channel_type,
        input.description.as_deref(),
        None,
    )?
    .sign_with_keys(&keys)
    .map_err(|error| error.to_string())?;
    let journal_owner = owner.clone();
    let journal_relay = relay.clone();
    let request = input.clone();
    let (creation, _changes) = state
        .archive_db
        .with_conn(move |conn| {
            let state = app.state::<AppState>();
            let _transition = state
                .managed_agent_runtime_transition
                .lock()
                .map_err(|error| error.to_string())?;
            let _store = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            if journal::load(conn, &journal_owner, &journal_relay, request.request_id)?.is_none() {
                require_idle_runtimes(&app, &state, &journal_relay)?;
            }
            let creation = journal::prepare(
                conn,
                &journal_owner,
                &journal_relay,
                &request,
                &event.as_json(),
            )?;
            Ok((creation, changes))
        })
        .await?;
    let completed = creation.result_json.is_some();
    let event = nostr::Event::from_json(&creation.event_json).map_err(|error| error.to_string())?;
    event.verify().map_err(|error| error.to_string())?;
    if event.pubkey != keys.public_key() {
        return Err("saved creation belongs to a different signing identity".into());
    }
    let api = crate::relay::relay_http_base_url(&relay);
    let filters = creation_filters(input.request_id, &event);
    let evidence =
        crate::relay::query_relay_at_with_keys(&state, &api, &filters, &keys, None).await?;
    let channel = match plan_creation(input.request_id, &event, &evidence, nostr::Timestamp::now())?
    {
        Some(channel) => channel,
        None => {
            if completed {
                return Err("creation was already delivered, but current ownership is not verified; no event will be retried".into());
            }
            let result =
                crate::relay::submit_signed_event_at_with_keys(&event, &state, &api, &keys).await;
            if let Err(error) = result {
                if !(error.contains("relay rejected event:")
                    && error.contains("duplicate: channel already exists"))
                {
                    return Err(format!("channel creation outcome not reconciled; resume the saved request: {error}"));
                }
            }
            let evidence =
                crate::relay::query_relay_at_with_keys(&state, &api, &filters, &keys, None).await?;
            reconcile_creation(input.request_id, &event, &evidence)?
                .ok_or("original creation and current ownership are not verified; resume later or inspect the relay before explicitly abandoning this local draft")?
        }
    };
    let receipt = serde_json::to_string(&channel).map_err(|error| error.to_string())?;
    if !completed {
        state
            .archive_db
            .with_conn(move |conn| {
                journal::complete(conn, &owner, &relay, input.request_id, &receipt)
            })
            .await?;
    }
    Ok(channel)
}

fn creation_filters(channel: uuid::Uuid, event: &nostr::Event) -> Vec<serde_json::Value> {
    use buzz_core_pkg::kind::{
        KIND_NIP29_CREATE_GROUP, KIND_NIP29_GROUP_ADMINS, KIND_NIP29_GROUP_METADATA,
    };
    vec![
        serde_json::json!({"kinds": [KIND_NIP29_CREATE_GROUP], "ids": [event.id.to_hex()], "limit": 1}),
        serde_json::json!({"kinds": [KIND_NIP29_GROUP_METADATA], "#d": [channel.to_string()], "limit": 1}),
        serde_json::json!({"kinds": [KIND_NIP29_GROUP_ADMINS], "#d": [channel.to_string()], "limit": 1}),
    ]
}

fn reconcile_creation(
    channel: uuid::Uuid,
    original: &nostr::Event,
    evidence: &[nostr::Event],
) -> Result<Option<ChannelInfo>, String> {
    use buzz_core_pkg::kind::{KIND_NIP29_GROUP_ADMINS, KIND_NIP29_GROUP_METADATA};
    for event in evidence {
        event.verify().map_err(|error| error.to_string())?;
    }
    if !evidence
        .iter()
        .any(|event| event.id == original.id && event.pubkey == original.pubkey)
    {
        return Ok(None);
    }
    let channel_id = channel.to_string();
    let is_channel = |event: &nostr::Event| {
        event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["d", channel_id.as_str()])
    };
    let owner = original.pubkey.to_hex();
    if !evidence.iter().any(|event| {
        event.kind.as_u16() as u32 == KIND_NIP29_GROUP_ADMINS
            && is_channel(event)
            && event
                .tags
                .iter()
                .any(|tag| tag.as_slice() == ["p", owner.as_str(), "owner"])
    }) {
        return Ok(None);
    }
    let Some(metadata) = evidence
        .iter()
        .find(|event| event.kind.as_u16() as u32 == KIND_NIP29_GROUP_METADATA && is_channel(event))
    else {
        return Ok(None);
    };
    crate::nostr_convert::channel_info_from_event(metadata, None, Some(true)).map(Some)
}

fn require_fresh_creation(event: &nostr::Event, now: nostr::Timestamp) -> Result<(), String> {
    if event.created_at.as_secs().abs_diff(now.as_secs()) > 900 {
        return Err("saved signature expired and creation is unverified; it will not be renewed automatically. Inspect the relay, then explicitly abandon this local draft if safe; folders and remote channels are never deleted".into());
    }
    Ok(())
}

fn plan_creation(
    channel: uuid::Uuid,
    original: &nostr::Event,
    evidence: &[nostr::Event],
    now: nostr::Timestamp,
) -> Result<Option<ChannelInfo>, String> {
    let receipt = reconcile_creation(channel, original, evidence)?;
    if receipt.is_none() {
        require_fresh_creation(original, now)?;
    }
    Ok(receipt)
}

/// Abandon a local draft only after the owner confirms reconciliation; retain all evidence and remote data.
#[tauri::command]
pub async fn abandon_project_channel_creation(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_owner: String,
    expected_relay: String,
    request_id: uuid::Uuid,
    confirmed_safe: bool,
) -> Result<(), String> {
    if !confirmed_safe {
        return Err("explicit owner confirmation of safe abandonment is required".into());
    }
    let _workspace = state.workspace_apply_lock.lock().await;
    let changes = std::sync::Arc::clone(&state.channel_workspace_change_lock)
        .write_owned()
        .await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    state
        .archive_db
        .with_conn(move |conn| {
            let _changes = changes;
            let state = app.state::<AppState>();
            let _transition = state
                .managed_agent_runtime_transition
                .lock()
                .map_err(|error| error.to_string())?;
            let _store = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            scope(&state, &expected_relay, &expected_owner)?;
            require_idle_runtimes(&app, &state, &relay)?;
            journal::abandon(conn, &owner, &relay, request_id)
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn evidence(owner: &Keys, relay: &Keys, channel: uuid::Uuid) -> Vec<nostr::Event> {
        let metadata = EventBuilder::new(Kind::Custom(39000), "")
            .tags([
                Tag::parse(["d", &channel.to_string()]).unwrap(),
                Tag::parse(["name", "synthetic"]).unwrap(),
                Tag::parse(["t", "stream"]).unwrap(),
            ])
            .sign_with_keys(relay)
            .unwrap();
        let admins = EventBuilder::new(Kind::Custom(39001), "")
            .tags([
                Tag::parse(["d", &channel.to_string()]).unwrap(),
                Tag::parse(["p", &owner.public_key().to_hex(), "owner"]).unwrap(),
            ])
            .sign_with_keys(relay)
            .unwrap();
        vec![metadata, admins]
    }

    #[test]
    fn channel_workspaces_reconcile_before_expiry_check_requires_original_creation_and_owner() {
        let owner = Keys::generate();
        let relay = Keys::generate();
        let foreign = Keys::generate();
        let channel = uuid::Uuid::new_v4();
        let original = crate::events::build_create_channel(
            channel,
            "synthetic",
            "private",
            "stream",
            None,
            None,
        )
        .unwrap()
        .custom_created_at(Timestamp::from_secs(1))
        .sign_with_keys(&owner)
        .unwrap();
        let mut proof = evidence(&owner, &relay, channel);
        assert!(plan_creation(channel, &original, &proof, Timestamp::from_secs(902)).is_err());
        assert!(
            plan_creation(channel, &original, &proof, Timestamp::from_secs(1))
                .unwrap()
                .is_none()
        );
        let foreign_event = crate::events::build_create_channel(
            channel,
            "synthetic",
            "private",
            "stream",
            None,
            None,
        )
        .unwrap()
        .custom_created_at(Timestamp::from_secs(1))
        .sign_with_keys(&foreign)
        .unwrap();
        proof.push(foreign_event);
        assert!(plan_creation(channel, &original, &proof, Timestamp::from_secs(902)).is_err());
        proof.push(original.clone());
        let recovered = plan_creation(channel, &original, &proof, Timestamp::from_secs(902))
            .unwrap()
            .unwrap();
        assert_eq!(recovered.id, channel.to_string());
        let mut revoked = evidence(&foreign, &relay, channel);
        revoked.push(original.clone());
        assert!(plan_creation(channel, &original, &revoked, Timestamp::from_secs(902)).is_err());
        assert!(plan_creation(
            uuid::Uuid::new_v4(),
            &original,
            &proof,
            Timestamp::from_secs(902)
        )
        .is_err());
    }
}
