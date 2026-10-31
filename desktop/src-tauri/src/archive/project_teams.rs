#[cfg(test)]
use buzz_agent_pkg::project_workspace::teams::validate;
pub use buzz_agent_pkg::project_workspace::teams::{
    authorize, load, ProjectMember, ProjectTeam, TeamUpdate,
};
use rusqlite::{Connection, TransactionBehavior};
#[cfg(test)]
use std::collections::HashSet;
use uuid::Uuid;
#[cfg(test)]
#[path = "project_teams_tests.rs"]
mod tests;

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    let applied = |conn: &Connection| -> Result<bool, String> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name='channel_project_teams_v1')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
    };
    if applied(conn)? {
        return Ok(());
    }
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !applied(&transaction)? {
        transaction.execute_batch("CREATE TABLE channel_project_teams (
            identity_pubkey TEXT NOT NULL, relay_url TEXT NOT NULL, channel_id TEXT NOT NULL, snapshot TEXT NOT NULL,
            PRIMARY KEY(identity_pubkey, relay_url, channel_id));
            CREATE TABLE channel_project_team_revisions (
            identity_pubkey TEXT NOT NULL, relay_url TEXT NOT NULL, channel_id TEXT NOT NULL, generation INTEGER NOT NULL,
            actor TEXT NOT NULL, snapshot TEXT NOT NULL, changed_at INTEGER NOT NULL,
            PRIMARY KEY(identity_pubkey, relay_url, channel_id, generation));
            INSERT INTO archive_migrations(name, applied_at) VALUES('channel_project_teams_v1', unixepoch());")
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(crate) fn prepare_check(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    binding_generation: i64,
    team_generation: i64,
) -> Result<(std::path::PathBuf, String), String> {
    super::workspace_provider_attempts::require_released(conn, relay)?;
    super::project_channel_creations::require_settled(conn, owner, relay)?;
    let binding = super::channel_workspaces::list(conn, owner, relay)?
        .into_iter()
        .find(|binding| binding.channel_id == channel)
        .ok_or("attach a project first")?;
    if binding.generation != binding_generation {
        return Err("project binding changed; reload before checking".into());
    }
    let team = load(conn, owner, relay, channel)?;
    let coordinator = team
        .coordinator_pubkey
        .as_deref()
        .ok_or("choose a coordinator first")?;
    for capability in ["read", "run_checks"] {
        authorize(
            &team,
            binding.generation,
            team_generation,
            coordinator,
            capability,
        )?;
    }
    Ok((
        super::channel_workspaces::validate_root(
            binding.root.as_deref().ok_or("attach a project first")?,
        )?,
        coordinator.into(),
    ))
}

pub(crate) fn set(
    conn: &Connection,
    update: TeamUpdate<'_>,
    next: ProjectTeam,
) -> Result<ProjectTeam, String> {
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    super::workspace_provider_attempts::require_released(&transaction, update.relay)?;
    buzz_agent_pkg::project_workspace::edits::require_reconciled(&transaction, update.relay)?;
    super::project_channel_creations::require_settled(&transaction, update.owner, update.relay)?;
    let binding = super::channel_workspaces::list(&transaction, update.owner, update.relay)?
        .into_iter()
        .find(|binding| binding.channel_id == update.channel && binding.root.is_some())
        .ok_or("attach a local project before configuring its team")?;
    let next = buzz_agent_pkg::project_workspace::teams::persist(
        &transaction,
        update,
        next,
        binding.generation,
    )?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(next)
}
