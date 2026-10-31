//! Owner-side task board commands.
//!
//! The owner plans a bounded task, assigns it to the coordinator, accepts a
//! reviewed delivery, pauses, resumes, or cancels. Agents advance tasks through
//! their native project tools; both paths use the same shared state machine.
//! Dispatch intents recorded by the state machine are published by
//! `project_dispatch`, never from here.

use super::channel_workspaces::scope;
use crate::{
    app_state::AppState,
    archive::project_tasks::{self, Actor, NewTask, Task, Transition},
};
use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

/// Board listing for one channel.
#[tauri::command]
pub async fn list_channel_project_tasks(
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
) -> Result<Vec<Task>, String> {
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let result = state
        .archive_db
        .with_conn(move |conn| project_tasks::list(conn, &owner, &relay, channel_id))
        .await?;
    scope(&state, &expected_relay, &expected_owner)?;
    Ok(result)
}

/// Owner task creation input.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerTaskInput {
    /// Short title.
    pub title: String,
    /// Bounded task statement.
    pub description: String,
    /// Inputs the assignee needs first.
    #[serde(default)]
    pub required_inputs: Vec<String>,
    /// Task ids that must complete first.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// `owner` (default) or an agent pubkey.
    #[serde(default)]
    pub return_to: Option<String>,
    /// Assign immediately to this team member.
    #[serde(default)]
    pub assignee_pubkey: Option<String>,
}

/// Plan one bounded task, optionally assigning it in the same step.
#[tauri::command]
pub async fn create_channel_project_task(
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
    input: OwnerTaskInput,
) -> Result<Task, String> {
    let _workspace = state.workspace_apply_lock.lock().await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    state
        .archive_db
        .with_conn(move |conn| {
            let transaction = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )
            .map_err(|error| error.to_string())?;
            let team = project_tasks::team_view(&transaction, &owner, &relay, channel_id)?
                .ok_or("attach a local project first")?;
            let return_to = input.return_to.clone().unwrap_or_else(|| "owner".into());
            let task = buzz_agent_pkg::project_workspace::tasks::create(
                &transaction,
                &owner,
                &relay,
                channel_id,
                Actor::Owner,
                &team.view(),
                NewTask {
                    title: &input.title,
                    description: &input.description,
                    required_inputs: &input.required_inputs,
                    dependencies: &input.dependencies,
                    return_to: &return_to,
                },
            )?;
            let task = match &input.assignee_pubkey {
                Some(assignee) => project_tasks::transition(
                    &transaction,
                    &owner,
                    &relay,
                    &task.task_id,
                    task.revision,
                    Actor::Owner,
                    &team.view(),
                    Transition::Assign { assignee },
                )?,
                None => task,
            };
            transaction.commit().map_err(|error| error.to_string())?;
            Ok(task)
        })
        .await
}

/// Owner transitions, named after the board actions.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OwnerTaskAction {
    /// Assign to a team member.
    Assign {
        /// Team member pubkey.
        assignee_pubkey: String,
    },
    /// Mark required inputs as provided.
    ProvideInput,
    /// Report a blocker.
    Block {
        /// Blocker text.
        blocker: String,
    },
    /// Accept a reviewed delivery.
    Accept,
    /// Pause the task.
    Pause,
    /// Resume the task.
    Resume,
    /// Cancel the task.
    Cancel,
}

/// Apply one owner transition, fenced by the task revision.
#[tauri::command]
pub async fn transition_channel_project_task(
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
    task_id: String,
    expected_revision: i64,
    action: OwnerTaskAction,
) -> Result<Task, String> {
    let _workspace = state.workspace_apply_lock.lock().await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    state
        .archive_db
        .with_conn(move |conn| {
            let transaction = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )
            .map_err(|error| error.to_string())?;
            let team = project_tasks::team_view(&transaction, &owner, &relay, channel_id)?
                .ok_or("attach a local project first")?;
            let existing = project_tasks::get(&transaction, &owner, &relay, &task_id)?
                .ok_or("unknown task")?;
            if existing.channel_id != channel_id {
                return Err("task belongs to another channel".into());
            }
            let transition = match &action {
                OwnerTaskAction::Assign { assignee_pubkey } => Transition::Assign {
                    assignee: assignee_pubkey,
                },
                OwnerTaskAction::ProvideInput => Transition::ProvideInput,
                OwnerTaskAction::Block { blocker } => Transition::Block { blocker },
                OwnerTaskAction::Accept => Transition::Accept,
                OwnerTaskAction::Pause => Transition::Pause,
                OwnerTaskAction::Resume => Transition::Resume,
                OwnerTaskAction::Cancel => Transition::Cancel,
            };
            let task = project_tasks::transition(
                &transaction,
                &owner,
                &relay,
                &task_id,
                expected_revision,
                Actor::Owner,
                &team.view(),
                transition,
            )?;
            transaction.commit().map_err(|error| error.to_string())?;
            Ok(task)
        })
        .await
}

/// Resolve a dispatch whose relay outcome was uncertain, after the owner checked
/// the channel: either the notification was observed (`observed_event_id`) or one
/// more attempt is authorized.
#[tauri::command]
pub async fn resolve_channel_project_task_dispatch(
    state: State<'_, AppState>,
    expected_relay: String,
    expected_owner: String,
    task_id: String,
    expected_revision: i64,
    observed_event_id: Option<String>,
) -> Result<Task, String> {
    let _workspace = state.workspace_apply_lock.lock().await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    state
        .archive_db
        .with_conn(move |conn| {
            project_tasks::resolve_uncertain_dispatch(
                conn,
                &owner,
                &relay,
                &task_id,
                expected_revision,
                observed_event_id.as_deref(),
            )?;
            project_tasks::get(conn, &owner, &relay, &task_id)?.ok_or_else(|| "unknown task".into())
        })
        .await
}
