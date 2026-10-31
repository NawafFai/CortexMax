use buzz_core_pkg::local_workspace::{ChannelWorkspaceBinding, MAX_CHANNEL_WORKSPACES};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    let is_applied = |connection: &Connection| -> Result<bool, String> {
        connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name = 'channel_workspaces_v1')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
    };
    if is_applied(conn)? {
        return Ok(());
    }
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !is_applied(&transaction)? {
        transaction
            .execute_batch(
                "CREATE TABLE channel_workspaces (
                identity_pubkey TEXT NOT NULL,
                relay_url TEXT NOT NULL,
                channel_id TEXT NOT NULL,
                generation INTEGER NOT NULL CHECK(generation > 0),
                root TEXT,
                PRIMARY KEY(identity_pubkey, relay_url, channel_id)
            );
            INSERT INTO archive_migrations(name, applied_at)
            VALUES ('channel_workspaces_v1', unixepoch());",
            )
            .map_err(|error| format!("channel workspace migration: {error}"))?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(crate) fn list(
    conn: &Connection,
    owner: &str,
    relay: &str,
) -> Result<Vec<ChannelWorkspaceBinding>, String> {
    let mut statement = conn
        .prepare(
            "SELECT channel_id, generation, root FROM channel_workspaces
         WHERE identity_pubkey = ?1 AND relay_url = ?2 ORDER BY channel_id LIMIT ?3",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![owner, relay, MAX_CHANNEL_WORKSPACES + 1], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut bindings = Vec::new();
    for row in rows {
        let (channel, generation, root) = row.map_err(|error| error.to_string())?;
        bindings.push(ChannelWorkspaceBinding {
            channel_id: Uuid::parse_str(&channel).map_err(|error| error.to_string())?,
            generation,
            root,
        });
    }
    if bindings.len() > MAX_CHANNEL_WORKSPACES {
        return Err("channel workspace limit exceeded".into());
    }
    Ok(bindings)
}

pub(crate) fn set(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    expected_generation: i64,
    root: Option<&str>,
) -> Result<ChannelWorkspaceBinding, String> {
    let root = root.map(validate_root).transpose()?;
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    super::project_channel_creations::require_settled(&transaction, owner, relay)?;
    let binding = set_in_transaction(
        &transaction,
        owner,
        relay,
        channel,
        expected_generation,
        root,
    )?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(binding)
}

pub(crate) fn set_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    owner: &str,
    relay: &str,
    channel: Uuid,
    expected_generation: i64,
    root: Option<PathBuf>,
) -> Result<ChannelWorkspaceBinding, String> {
    super::workspace_provider_attempts::require_released(transaction, relay)?;
    buzz_agent_pkg::project_workspace::edits::require_reconciled(transaction, relay)?;
    let previous: Option<i64> = transaction
        .query_row(
            "SELECT generation FROM channel_workspaces
         WHERE identity_pubkey = ?1 AND relay_url = ?2 AND channel_id = ?3",
            params![owner, relay, channel.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if previous.unwrap_or(0) != expected_generation {
        return Err("project binding changed; reload before saving".into());
    }
    if previous.is_none() && list(transaction, owner, relay)?.len() >= MAX_CHANNEL_WORKSPACES {
        return Err("channel workspace limit reached".into());
    }
    let generation = expected_generation
        .checked_add(1)
        .filter(|generation| *generation > 0)
        .ok_or("invalid project binding generation")?;
    let root = root.map(|path| path.to_string_lossy().into_owned());
    transaction.execute(
        "INSERT INTO channel_workspaces(identity_pubkey, relay_url, channel_id, generation, root)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(identity_pubkey, relay_url, channel_id)
         DO UPDATE SET generation = excluded.generation, root = excluded.root",
        params![owner, relay, channel.to_string(), generation, root],
    ).map_err(|error| error.to_string())?;
    Ok(ChannelWorkspaceBinding {
        channel_id: channel,
        generation,
        root,
    })
}

pub(crate) fn validate_root(raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if raw.len() > 4096
        || !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("select an absolute project folder without parent traversal".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("project folder unavailable: {error}"))?;
    if canonical != path || !canonical.is_dir() {
        return Err("select the canonical project folder, not a symlink".into());
    }
    std::fs::read_dir(&canonical).map_err(|error| format!("project folder unreadable: {error}"))?;
    Ok(canonical)
}

#[cfg(test)]
#[path = "channel_workspaces_tests.rs"]
mod tests;
