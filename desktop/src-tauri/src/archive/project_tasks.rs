//! Desktop wrapper over the shared task board (`buzz_agent::project_workspace::tasks`).
//!
//! The state machine, its tables and its tests live in the shared crate so the
//! desktop (owner actions, dispatch) and the native agent tools advance the same
//! authoritative record. This module only adapts it to the desktop archive:
//! migration registration and the team view the state machine needs.

pub use buzz_agent_pkg::project_workspace::tasks::{
    begin_dispatch, confirm_dispatch, count_in_state, dispatch_attempts, dispatch_backlog, get,
    list, mark_dispatch_uncertain, note_dispatch, resolve_uncertain_dispatch, transition, Actor,
    NewTask, Task, TeamView, Transition,
};
use rusqlite::Connection;
use uuid::Uuid;

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    buzz_agent_pkg::project_workspace::tasks::migrate(conn)
}

/// Owned team facts for the state machine, loaded from the saved team and the
/// current binding. `None` when the channel has no attached project.
pub(crate) struct OwnedTeamView {
    pub coordinator: Option<String>,
    pub members: Vec<(String, Vec<String>)>,
    pub paused: bool,
    pub binding_generation: i64,
    pub team_generation: i64,
}

impl OwnedTeamView {
    pub(crate) fn view(&self) -> TeamView<'_> {
        TeamView {
            coordinator: self.coordinator.as_deref(),
            members: &self.members,
            paused: self.paused,
            binding_generation: self.binding_generation,
            team_generation: self.team_generation,
        }
    }
}

/// Load the team view for a channel with an attached project.
pub(crate) fn team_view(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
) -> Result<Option<OwnedTeamView>, String> {
    let Some(binding) = super::channel_workspaces::list(conn, owner, relay)?
        .into_iter()
        .find(|binding| binding.channel_id == channel && binding.root.is_some())
    else {
        return Ok(None);
    };
    let team = super::project_teams::load(conn, owner, relay, channel)?;
    Ok(Some(OwnedTeamView {
        coordinator: team.coordinator_pubkey.clone(),
        members: team
            .members
            .iter()
            .map(|member| (member.pubkey.clone(), member.capabilities.clone()))
            .collect(),
        paused: team.paused,
        binding_generation: binding.generation,
        team_generation: team.generation,
    }))
}
