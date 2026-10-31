//! Owner approval queue for actions the harness gate refused.
//!
//! Deciding a request is not executing it. Approving an `execute` request may add
//! its exact command line to the team's approved checks (a new team generation);
//! the agent's next turn then runs with that grant and may retry. Nothing here
//! runs a command, touches the project, or messages an agent.

use super::channel_workspaces::scope;
use crate::{
    app_state::AppState,
    archive::project_teams::{self, TeamUpdate},
};
use buzz_agent_pkg::project_workspace::approvals::{self, ApprovalRequest};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

/// List approval requests for the community, optionally one channel. Pending first.
#[tauri::command]
pub async fn list_project_approval_requests(
    state: State<'_, AppState>,
    expected_relay: String,
    expected_owner: String,
    channel_id: Option<Uuid>,
) -> Result<Vec<ApprovalRequest>, String> {
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let result = state
        .archive_db
        .with_conn(move |conn| approvals::list(conn, &owner, &relay, channel_id))
        .await?;
    scope(&state, &expected_relay, &expected_owner)?;
    Ok(result)
}

/// Record the owner's decision. With `add_to_approved_checks`, an approved
/// `execute` request's command line joins the team's approved checks atomically
/// with the decision, fenced by the caller's team generation.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn decide_project_approval_request(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_relay: String,
    expected_owner: String,
    request_id: String,
    approve: bool,
    note: Option<String>,
    add_to_approved_checks: bool,
    expected_team_generation: Option<i64>,
) -> Result<ApprovalRequest, String> {
    let _workspace = state.workspace_apply_lock.lock().await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let result = state
        .archive_db
        .with_conn(move |conn| {
            let state = app.state::<AppState>();
            let _store = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let transaction = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )
            .map_err(|error| error.to_string())?;
            let decided = approvals::decide(
                &transaction,
                &owner,
                &relay,
                &request_id,
                approve,
                note.as_deref(),
            )?;
            if approve && add_to_approved_checks {
                if decided.category != "execute" {
                    return Err("only refused commands can be added to the approved checks".into());
                }
                let expected_generation = expected_team_generation
                    .ok_or("team generation is required to add an approved check")?;
                let mut team =
                    project_teams::load(&transaction, &owner, &relay, decided.channel_id)?;
                if !team
                    .approved_checks
                    .iter()
                    .any(|command| command == &decided.summary)
                {
                    team.approved_checks.push(decided.summary.clone());
                }
                let binding =
                    crate::archive::channel_workspaces::list(&transaction, &owner, &relay)?
                        .into_iter()
                        .find(|binding| {
                            binding.channel_id == decided.channel_id && binding.root.is_some()
                        })
                        .ok_or("the project was detached; the check was not approved")?;
                let agents =
                    crate::managed_agents::storage::load_project_agent_metadata(&app, &owner)?;
                let available = agents.iter().map(|agent| agent.pubkey.clone()).collect();
                team.generation = 0;
                team.binding_generation = 0;
                buzz_agent_pkg::project_workspace::teams::persist(
                    &transaction,
                    TeamUpdate {
                        owner: &owner,
                        relay: &relay,
                        channel: decided.channel_id,
                        actor: &owner,
                        expected_generation,
                        expected_binding_generation: binding.generation,
                        available: &available,
                    },
                    team,
                    binding.generation,
                )?;
            }
            transaction.commit().map_err(|error| error.to_string())?;
            Ok(decided)
        })
        .await?;
    scope(&state, &expected_relay, &expected_owner)?;
    Ok(result)
}
