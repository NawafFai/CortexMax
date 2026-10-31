use buzz_core_pkg::local_workspace::{ChannelWorkspaceBinding, ChannelWorkspaceSnapshot};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::archive::channel_workspaces;

/// Select an existing local project directory without writing into it.
#[tauri::command]
pub async fn pick_channel_workspace_folder(app: AppHandle) -> Result<Option<String>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = sender.send(path);
    });
    let selected = receiver.await.map_err(|_| "folder picker unavailable")?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected
        .as_path()
        .ok_or("folder picker returned an invalid path")?
        .to_str()
        .ok_or("project folder path must be valid Unicode")?
        .to_string();
    tokio::task::spawn_blocking(move || {
        channel_workspaces::validate_root(&path)?;
        Ok(Some(path))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// A local binding and deliberately separate filesystem and execution diagnostics.
#[derive(Serialize)]
pub struct ChannelWorkspaceStatus {
    /// Provider attempts whose externally executed effects have no durable receipt.
    pub pending_provider_attempts:
        Vec<crate::archive::workspace_provider_attempts::PendingProviderAttempt>,
    /// Time of this desktop-only filesystem check, in Unix seconds.
    pub checked_at: u64,
    /// Missing means this channel has never had a local project binding.
    pub binding: Option<ChannelWorkspaceBinding>,
    /// This device could open the directory; not proof of agent access.
    pub folder_readable: bool,
    /// Current filesystem failure, without converting errors into readiness.
    pub folder_error: Option<String>,
    /// No runtime currently has a verified project enforcement contract.
    pub execution_ready: bool,
    /// Actionable explanation of the execution gate.
    pub execution_blocker: String,
}

pub(crate) fn scope(
    state: &AppState,
    expected_relay: &str,
    expected_owner: &str,
) -> Result<(String, String), String> {
    let owner = state.signing_keys()?.public_key().to_hex();
    let relay =
        buzz_core_pkg::relay::normalize_relay_url(&crate::relay::relay_ws_url_with_override(state))
            .map_err(|error| error.to_string())?;
    let expected = buzz_core_pkg::relay::normalize_relay_url(expected_relay)
        .map_err(|error| error.to_string())?;
    if owner != expected_owner || relay != expected {
        return Err("community or identity changed; reload the channel project".into());
    }
    Ok((owner, relay))
}

/// Read a device-local binding without publishing the filesystem path.
#[tauri::command]
pub async fn get_channel_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
) -> Result<ChannelWorkspaceStatus, String> {
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let status = state
        .archive_db
        .with_conn(move |conn| {
            let binding = channel_workspaces::list(conn, &owner, &relay)?
                .into_iter()
                .find(|binding| binding.channel_id == channel_id);
            let folder = binding
                .as_ref()
                .and_then(|binding| binding.root.as_deref())
                .map(channel_workspaces::validate_root);
            let team = crate::archive::project_teams::load(conn, &owner, &relay, channel_id)?;
            let readiness = super::channel_project_team::readiness(
                &app, conn, &owner, &relay, channel_id, &team,
            )?;
            // Ready means the ladder reached execution authorization: attached,
            // runtimes configured, access verified by inspection, not paused.
            let execution_ready =
                matches!(readiness.state.as_str(), "execution_authorized" | "task_running");
            let execution_blocker = if execution_ready {
                String::new()
            } else if readiness.blockers.is_empty() {
                format!("Readiness is `{}`; run the isolated inspection and resume dispatch before assigning work.", readiness.state)
            } else {
                readiness.blockers.join(" ")
            };
            Ok(ChannelWorkspaceStatus {
                pending_provider_attempts: crate::archive::workspace_provider_attempts::pending(
                    conn, &owner, &relay,
                )?,
                checked_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_secs(),
                folder_readable: matches!(&folder, Some(Ok(_))),
                folder_error: folder.and_then(Result::err),
                binding,
                execution_ready,
                execution_blocker,
            })
        })
        .await?;
    scope(&state, &expected_relay, &expected_owner)?;
    Ok(status)
}

/// Attach, rebind or detach atomically; reject stale saves and unresolved runtimes.
#[tauri::command]
pub async fn set_channel_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
    expected_generation: i64,
    root: Option<String>,
) -> Result<ChannelWorkspaceBinding, String> {
    let _workspace = state.workspace_apply_lock.lock().await;
    let changes = std::sync::Arc::clone(&state.channel_workspace_change_lock)
        .write_owned()
        .await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let events = crate::relay::query_relay(&state, &[serde_json::json!({
        "kinds": [buzz_core_pkg::kind::KIND_NIP29_GROUP_MEMBERS], "#d": [channel_id.to_string()], "limit": 1
    })]).await?;
    if !events.iter().any(|event| {
        event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().map(String::as_str) == Some("p") && values.get(1) == Some(&owner)
        })
    }) {
        return Err("channel membership could not be verified; join the channel and retry".into());
    }
    scope(&state, &expected_relay, &expected_owner)?;
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
            crate::archive::workspace_provider_attempts::require_released(conn, &relay)?;
            channel_workspaces::set(
                conn,
                &owner,
                &relay,
                channel_id,
                expected_generation,
                root.as_deref(),
            )
        })
        .await
}

pub(crate) fn require_idle_runtimes(
    app: &AppHandle,
    state: &AppState,
    relay: &str,
) -> Result<(), String> {
    let runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    if runtimes.keys().any(|key| key.relay_url == relay) {
        return Err("stop this community's local runtimes before changing its project binding; running work will not be redirected".into());
    }
    crate::managed_agents::storage::assert_no_project_runtime_receipts(app, relay)?;
    for record in crate::managed_agents::load_managed_agents(app)? {
        if record.runtime_pid.is_some() || record.backend_agent_id.is_some() {
            let effective_relay = crate::relay::effective_agent_relay_url(&record.relay_url, relay);
            let record_relay = buzz_core_pkg::relay::normalize_relay_url(&effective_relay)
                .map_err(|error| error.to_string())?;
            if record_relay == relay {
                return Err("runtime ownership remains unresolved; release previous local or remote writers before changing the binding".into());
            }
        }
    }
    Ok(())
}

pub(crate) fn launch_snapshot(
    app: &AppHandle,
    owner: &str,
    relay: &str,
) -> Result<ChannelWorkspaceSnapshot, String> {
    let owner = owner.to_string();
    let relay =
        buzz_core_pkg::relay::normalize_relay_url(relay).map_err(|error| error.to_string())?;
    std::thread::scope(|threads| {
        threads
            .spawn(move || {
                let state = app.state::<AppState>();
                tauri::async_runtime::block_on(state.archive_db.with_conn(move |conn| {
                    crate::archive::workspace_provider_attempts::require_released(conn, &relay)?;
                    Ok(ChannelWorkspaceSnapshot {
                        version: 1,
                        bindings: channel_workspaces::list(conn, &owner, &relay)?,
                        relay_url: relay,
                        owner_pubkey: owner,
                    })
                }))
            })
            .join()
            .map_err(|_| "workspace snapshot reader failed".to_string())?
    })
}

/// Channels with an attached root whose saved team lists `agent_pubkey`.
///
/// This is the per-agent membership fact the launch path uses to decide whether
/// a runtime carries project authority at all; an agent on no bound team is
/// launched conversation-only regardless of what other channels attached.
pub(crate) fn bound_team_channels(
    app: &AppHandle,
    owner: &str,
    relay: &str,
    agent_pubkey: &str,
) -> Result<Vec<Uuid>, String> {
    let owner = owner.to_string();
    let relay =
        buzz_core_pkg::relay::normalize_relay_url(relay).map_err(|error| error.to_string())?;
    let agent = agent_pubkey.to_string();
    std::thread::scope(|threads| {
        threads
            .spawn(move || {
                let state = app.state::<AppState>();
                tauri::async_runtime::block_on(state.archive_db.with_conn(move |conn| {
                    let mut channels = Vec::new();
                    for binding in channel_workspaces::list(conn, &owner, &relay)? {
                        if binding.root.is_none() {
                            continue;
                        }
                        let team = crate::archive::project_teams::load(
                            conn,
                            &owner,
                            &relay,
                            binding.channel_id,
                        )?;
                        if team.members.iter().any(|member| member.pubkey == agent) {
                            channels.push(binding.channel_id);
                        }
                    }
                    Ok(channels)
                }))
            })
            .join()
            .map_err(|_| "team membership reader failed".to_string())?
    })
}

/// Record the owner's explicit confirmation of external writer release, not a stop operation.
#[tauri::command]
pub async fn confirm_channel_workspace_writer_release(
    state: State<'_, AppState>,
    expected_relay: String,
    expected_owner: String,
    attempt_id: String,
    writer_released: bool,
) -> Result<(), String> {
    if !writer_released {
        return Err("explicit confirmation of external writer release is required".into());
    }
    let _workspace = state.workspace_apply_lock.lock().await;
    let _changes = state.channel_workspace_change_lock.write().await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    state
        .archive_db
        .with_conn(move |conn| {
            crate::archive::workspace_provider_attempts::resolve(conn, &owner, &relay, &attempt_id)
        })
        .await
}
