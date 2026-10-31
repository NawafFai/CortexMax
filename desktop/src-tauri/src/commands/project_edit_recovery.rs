use super::channel_workspaces::{require_idle_runtimes, scope};
use crate::app_state::AppState;
use tauri::{AppHandle, Manager, State};

/// Read unresolved local edit receipts, without interpreting a model's completion claim.
#[tauri::command]
pub async fn get_pending_project_edits(
    state: State<'_, AppState>,
    expected_owner: String,
    expected_relay: String,
) -> Result<Vec<buzz_agent_pkg::project_workspace::edits::PendingEdit>, String> {
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let pending = state
        .archive_db
        .with_conn(move |conn| {
            buzz_agent_pkg::project_workspace::edits::pending(conn, &owner, &relay)
        })
        .await?;
    scope(&state, &expected_relay, &expected_owner)?;
    Ok(pending)
}

/// Reconcile only after explicit owner inspection and writer release; never change project files.
#[tauri::command]
pub async fn reconcile_project_edit(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_owner: String,
    expected_relay: String,
    operation_id: String,
    expected_current_sha256: String,
    confirmed_writer_stopped: bool,
) -> Result<(), String> {
    if !confirmed_writer_stopped {
        return Err("independently confirm the previous writer stopped and inspect the current file before reconciliation".into());
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
            buzz_agent_pkg::project_workspace::edits::reconcile(
                conn,
                &owner,
                &relay,
                &operation_id,
                &expected_current_sha256,
            )
        })
        .await
}
