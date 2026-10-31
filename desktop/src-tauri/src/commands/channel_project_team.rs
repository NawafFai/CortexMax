use super::channel_workspaces::scope;
use crate::{
    app_state::AppState,
    archive::{
        channel_workspaces,
        project_teams::{self, ProjectMember, ProjectTeam, TeamUpdate},
    },
    managed_agents::{storage::ProjectAgentMetadata, ProjectEnforcementSummary},
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

/// Public identity metadata only; no identity keys or launcher environment.
#[derive(Serialize)]
pub struct AvailableProjectAgent {
    /// Existing public key.
    pub pubkey: String,
    /// Existing display name.
    pub display_name: String,
    /// Configured runtime identifier, not proof of support.
    pub runtime_id: Option<String>,
    /// Whether this is a local managed runtime.
    pub local: bool,
    /// Verified project enforcement for the configured runtime, from the catalog.
    pub enforcement: ProjectEnforcementSummary,
}

/// One saved grant and its current public identity label.
#[derive(Serialize)]
pub struct ProjectTeamMember {
    /// Saved bounded grant.
    #[serde(flatten)]
    pub grant: ProjectMember,
    /// Public name; the public key is shown when an identity is no longer available.
    pub display_name: String,
}

/// Project team state with structured readiness.
#[derive(Serialize)]
pub struct ChannelProjectTeamStatus {
    /// Durable authority revision.
    pub generation: i64,
    /// Current binding revision observed by this team editor, not a permission grant.
    pub binding_generation: i64,
    /// Coordinator chosen from existing agents.
    pub coordinator_pubkey: Option<String>,
    /// Project-local members; removal never deletes global identities.
    pub members: Vec<ProjectTeamMember>,
    /// Existing identities whose owner attestation matches the active owner.
    pub available_agents: Vec<AvailableProjectAgent>,
    /// Owner-controlled pause state.
    pub paused: bool,
    /// Exact command lines approved for `run_checks` holders.
    pub approved_checks: Vec<String>,
    /// Structured readiness with one entry per established fact.
    pub readiness: super::channel_project_readiness::ProjectReadiness,
    /// Messages of every failed readiness check (kept for existing callers).
    pub execution_blockers: Vec<String>,
    /// Grantable roles, from the shared vocabulary.
    pub roles: Vec<String>,
    /// Grantable capabilities, from the shared vocabulary.
    pub capabilities: Vec<String>,
}

/// Full readiness for a channel; used by the workspace status.
pub(crate) fn readiness(
    app: &AppHandle,
    conn: &rusqlite::Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    team: &ProjectTeam,
) -> Result<super::channel_project_readiness::ProjectReadiness, String> {
    let agents = crate::managed_agents::storage::load_project_agent_metadata(app, owner)?;
    super::channel_project_readiness::compute(app, conn, owner, relay, channel, team, &agents)
}

fn status(
    app: &AppHandle,
    conn: &rusqlite::Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    agents: &[ProjectAgentMetadata],
) -> Result<ChannelProjectTeamStatus, String> {
    let team = project_teams::load(conn, owner, relay, channel)?;
    let readiness =
        super::channel_project_readiness::compute(app, conn, owner, relay, channel, &team, agents)?;
    Ok(ChannelProjectTeamStatus {
        generation: team.generation,
        binding_generation: channel_workspaces::list(conn, owner, relay)?
            .into_iter()
            .find(|binding| binding.channel_id == channel)
            .map(|binding| binding.generation)
            .unwrap_or(0),
        coordinator_pubkey: team.coordinator_pubkey,
        paused: team.paused,
        approved_checks: team.approved_checks,
        members: team
            .members
            .into_iter()
            .map(|grant| ProjectTeamMember {
                display_name: agents
                    .iter()
                    .find(|agent| agent.pubkey == grant.pubkey)
                    .map(|agent| agent.name.clone())
                    .unwrap_or_else(|| grant.pubkey.clone()),
                grant,
            })
            .collect(),
        available_agents: agents
            .iter()
            .map(|agent| AvailableProjectAgent {
                pubkey: agent.pubkey.clone(),
                display_name: agent.name.clone(),
                runtime_id: agent.runtime.clone(),
                local: agent.local,
                enforcement: crate::managed_agents::project_enforcement::summary_for_runtime_id(
                    agent.runtime.as_deref(),
                ),
            })
            .collect(),
        execution_blockers: readiness.blockers.clone(),
        readiness,
        roles: buzz_agent_pkg::project_workspace::teams::PROJECT_ROLES
            .iter()
            .map(|role| role.to_string())
            .collect(),
        capabilities: buzz_agent_pkg::project_workspace::teams::PROJECT_CAPABILITIES
            .iter()
            .map(|capability| capability.to_string())
            .collect(),
    })
}

/// Read project-local authority without touching global agent identities or secrets.
#[tauri::command]
pub async fn get_channel_project_team(
    app: AppHandle,
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
) -> Result<ChannelProjectTeamStatus, String> {
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let result = state
        .archive_db
        .with_conn(move |conn| {
            let state = app.state::<AppState>();
            let _store = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let agents = crate::managed_agents::storage::load_project_agent_metadata(&app, &owner)?;
            status(&app, conn, &owner, &relay, channel_id, &agents)
        })
        .await?;
    scope(&state, &expected_relay, &expected_owner)?;
    Ok(result)
}

async fn require_channel_owner(
    state: &AppState,
    channel_id: Uuid,
    owner: &str,
) -> Result<(), String> {
    let events = crate::relay::query_relay(state, &[serde_json::json!({"kinds": [buzz_core_pkg::kind::KIND_NIP29_GROUP_ADMINS], "#d": [channel_id.to_string()], "limit": 1})]).await?;
    if !events.iter().any(|event| {
        event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().is_some_and(|value| value == "p")
                && values.get(1).map(String::as_str) == Some(owner)
                && values.iter().skip(2).any(|role| role == "owner")
        })
    }) {
        return Err(
            "current channel ownership could not be verified; project grants were not changed"
                .into(),
        );
    }
    Ok(())
}

/// Replace local grants atomically; reject stale edits.
///
/// Running runtimes are not stopped: every native tool call re-reads authority,
/// and harness-gated sessions re-resolve their grants before each turn, so a
/// grant change retires stale sessions instead of redirecting them. Changing the
/// project *folder* still requires idle runtimes (`set_channel_workspace`).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn set_channel_project_team(
    app: AppHandle,
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
    expected_generation: i64,
    expected_binding_generation: i64,
    coordinator_pubkey: Option<String>,
    members: Vec<ProjectMember>,
    paused: bool,
    approved_checks: Option<Vec<String>>,
) -> Result<ChannelProjectTeamStatus, String> {
    let _workspace = state.workspace_apply_lock.lock().await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    require_channel_owner(&state, channel_id, &owner).await?;
    state
        .archive_db
        .with_conn(move |conn| {
            let state = app.state::<AppState>();
            let _store = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            scope(&state, &expected_relay, &expected_owner)?;
            let agents = crate::managed_agents::storage::load_project_agent_metadata(&app, &owner)?;
            let available = agents.iter().map(|agent| agent.pubkey.clone()).collect();
            let previous = project_teams::load(conn, &owner, &relay, channel_id)?;
            project_teams::set(
                conn,
                TeamUpdate {
                    owner: &owner,
                    relay: &relay,
                    channel: channel_id,
                    actor: &owner,
                    expected_generation,
                    expected_binding_generation,
                    available: &available,
                },
                ProjectTeam {
                    generation: 0,
                    binding_generation: 0,
                    coordinator_pubkey,
                    members,
                    paused,
                    approved_checks: approved_checks.unwrap_or(previous.approved_checks),
                },
            )?;
            status(&app, conn, &owner, &relay, channel_id, &agents)
        })
        .await
}

/// Pause or resume project dispatch without touching grants or running processes.
///
/// Pausing stops new assignments and task starts and makes every native tool
/// call and gated turn refuse project work on its next authority check; it does
/// not kill an in-flight process and never deletes work.
#[tauri::command]
pub async fn set_channel_project_paused(
    app: AppHandle,
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
    expected_generation: i64,
    paused: bool,
) -> Result<ChannelProjectTeamStatus, String> {
    let _workspace = state.workspace_apply_lock.lock().await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    require_channel_owner(&state, channel_id, &owner).await?;
    state
        .archive_db
        .with_conn(move |conn| {
            let state = app.state::<AppState>();
            let _store = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            scope(&state, &expected_relay, &expected_owner)?;
            let agents = crate::managed_agents::storage::load_project_agent_metadata(&app, &owner)?;
            let available = agents.iter().map(|agent| agent.pubkey.clone()).collect();
            let previous = project_teams::load(conn, &owner, &relay, channel_id)?;
            let binding_generation = channel_workspaces::list(conn, &owner, &relay)?
                .into_iter()
                .find(|binding| binding.channel_id == channel_id && binding.root.is_some())
                .map(|binding| binding.generation)
                .ok_or("attach a local project first")?;
            project_teams::set(
                conn,
                TeamUpdate {
                    owner: &owner,
                    relay: &relay,
                    channel: channel_id,
                    actor: &owner,
                    expected_generation,
                    expected_binding_generation: binding_generation,
                    available: &available,
                },
                ProjectTeam {
                    generation: 0,
                    binding_generation: 0,
                    coordinator_pubkey: previous.coordinator_pubkey,
                    members: previous.members,
                    paused,
                    approved_checks: previous.approved_checks,
                },
            )?;
            status(&app, conn, &owner, &relay, channel_id, &agents)
        })
        .await
}
