use super::channel_workspaces::scope;
use crate::{
    app_state::AppState,
    archive::project_teams,
    project_check_runtime::{self, ProjectCheckEvidence},
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

/// A scoped native check result, not an agent/provider readiness certificate.
#[derive(Serialize)]
pub struct ChannelProjectCheck {
    /// Channel whose immutable binding was inspected.
    pub channel_id: Uuid,
    /// Binding generation held for the whole operation.
    pub binding_generation: i64,
    /// Authority generation held for the whole operation.
    pub team_generation: i64,
    /// Check completion time in Unix seconds.
    pub checked_at: u64,
    /// Actual restricted child process results.
    pub evidence: ProjectCheckEvidence,
}

/// Run a fixed read-only native check; never launch or authenticate a model provider.
#[tauri::command]
pub async fn inspect_channel_project(
    app: AppHandle,
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
    expected_binding_generation: i64,
    expected_team_generation: i64,
) -> Result<ChannelProjectCheck, String> {
    let changes = std::sync::Arc::clone(&state.channel_workspace_change_lock)
        .read_owned()
        .await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let events = crate::relay::query_relay(&state, &[serde_json::json!({
        "kinds": [buzz_core_pkg::kind::KIND_NIP29_GROUP_ADMINS], "#d": [channel_id.to_string()], "limit": 1
    })]).await?;
    if !events.iter().any(|event| {
        event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().is_some_and(|value| value == "p")
                && values.get(1) == Some(&owner)
                && values.iter().skip(2).any(|role| role == "owner")
        })
    }) {
        return Err("current channel ownership could not be verified".into());
    }
    tauri::async_runtime::spawn(async move {
        let _changes = changes;
        let state = app.state::<AppState>();
        scope(&state, &expected_relay, &expected_owner)?;
        let store_owner = owner.clone();
        let store_relay = relay.clone();
        let archive_app = app.clone();
        let root = state
            .archive_db
            .with_conn(move |conn| {
                let state = archive_app.state::<AppState>();
                let _transition = state
                    .managed_agent_runtime_transition
                    .lock()
                    .map_err(|error| error.to_string())?;
                let _store = state
                    .managed_agents_store_lock
                    .lock()
                    .map_err(|error| error.to_string())?;
                let (root, coordinator) = project_teams::prepare_check(
                    conn,
                    &owner,
                    &relay,
                    channel_id,
                    expected_binding_generation,
                    expected_team_generation,
                )?;
                let agents = crate::managed_agents::storage::load_project_agent_metadata(
                    &archive_app,
                    &owner,
                )?;
                if !agents.iter().any(|agent| agent.pubkey == coordinator) {
                    return Err("coordinator identity ownership is no longer verified".into());
                }
                Ok(root)
            })
            .await?;
        let evidence = project_check_runtime::inspect(root).await?;
        scope(&state, &expected_relay, &expected_owner)?;
        let checked_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs();
        // Persist the evidence under the generations it was taken for, so the
        // readiness check can show when access was last verified and drop it the
        // moment the binding or team moves.
        let stored = crate::archive::project_readiness::StoredInspection {
            binding_generation: expected_binding_generation,
            team_generation: expected_team_generation,
            checked_at: checked_at as i64,
            actual_cwd: evidence.actual_cwd.clone(),
            branch: evidence.branch.clone(),
            revision: evidence.revision.clone(),
            dirty: evidence.dirty,
            runtime: evidence.runtime.to_string(),
        };
        state
            .archive_db
            .with_conn(move |conn| {
                crate::archive::project_readiness::store(
                    conn,
                    &store_owner,
                    &store_relay,
                    channel_id,
                    &stored,
                )
            })
            .await?;
        Ok(ChannelProjectCheck {
            channel_id,
            binding_generation: expected_binding_generation,
            team_generation: expected_team_generation,
            checked_at,
            evidence,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
