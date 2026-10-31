use rusqlite::{params, Connection};
use serde::Serialize;

/// A provider invocation whose external outcome still needs reconciliation.
#[derive(Serialize)]
pub struct PendingProviderAttempt {
    /// Exact attempt to reconcile; a new attempt receives a different id.
    pub id: String,
    /// Public identity of the affected agent.
    pub agent_pubkey: String,
}

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    let is_applied = |connection: &Connection| -> Result<bool, String> {
        connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name = 'workspace_provider_attempts_v1')",
        [], |row| row.get(0),
    ).map_err(|error| error.to_string())
    };
    if is_applied(conn)? {
        return Ok(());
    }
    let transaction =
        rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
    if !is_applied(&transaction)? {
        transaction.execute_batch("CREATE TABLE workspace_provider_attempts (
            identity_pubkey TEXT NOT NULL, relay_url TEXT NOT NULL, agent_pubkey TEXT NOT NULL,
            id TEXT NOT NULL, PRIMARY KEY(identity_pubkey, relay_url, agent_pubkey)
        ); CREATE INDEX workspace_provider_attempts_relay_agent ON workspace_provider_attempts(relay_url, agent_pubkey);
        INSERT INTO archive_migrations(name, applied_at) VALUES ('workspace_provider_attempts_v1', unixepoch());")
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(crate) fn pending(
    conn: &Connection,
    owner: &str,
    relay: &str,
) -> Result<Vec<PendingProviderAttempt>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, agent_pubkey FROM workspace_provider_attempts
        WHERE identity_pubkey = ?1 AND relay_url = ?2 ORDER BY id LIMIT 257",
        )
        .map_err(|error| error.to_string())?;
    let attempts = statement
        .query_map(params![owner, relay], |row| {
            Ok(PendingProviderAttempt {
                id: row.get(0)?,
                agent_pubkey: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if attempts.len() > 256 {
        return Err("pending provider attempt limit exceeded".into());
    }
    Ok(attempts)
}

pub(crate) fn begin(
    conn: &Connection,
    owner: &str,
    relay: &str,
    agent: &str,
) -> Result<String, String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
    let attempts = pending(&transaction, owner, relay)?;
    let conflict: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM workspace_provider_attempts WHERE relay_url = ?1 AND agent_pubkey = ?2)",
        params![relay, agent], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if conflict {
        return Err("previous provider outcome is uncertain; return to the identity that started it and verify writer release in the channel Project view before retrying".into());
    }
    if attempts.len() >= 256 {
        return Err("pending provider attempt limit reached".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    transaction
        .execute(
            "INSERT INTO workspace_provider_attempts(identity_pubkey, relay_url, agent_pubkey, id)
        VALUES (?1, ?2, ?3, ?4)",
            params![owner, relay, agent, id],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(id)
}

pub(crate) fn resolve(conn: &Connection, owner: &str, relay: &str, id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "DELETE FROM workspace_provider_attempts
        WHERE identity_pubkey = ?1 AND relay_url = ?2 AND id = ?3",
            params![owner, relay, id],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("provider attempt changed or belongs to another scope; reload".into());
    }
    Ok(())
}

pub(crate) fn require_released(conn: &Connection, relay: &str) -> Result<(), String> {
    let conflict: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM workspace_provider_attempts WHERE relay_url = ?1)",
            [relay],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if conflict {
        return Err("provider execution outcome is uncertain in this community; return to the identity that started it and verify remote writer release before changing bindings or starting work".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::store::open_archive_db;

    #[test]
    fn channel_workspaces_attempt_migration_preserves_evidence_and_rolls_back_failure() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER);
            CREATE TABLE evidence(value TEXT); INSERT INTO evidence VALUES ('preserved');",
            )
            .unwrap();
        migrate(&connection).unwrap();
        let attempt = begin(&connection, "owner", "relay", "agent").unwrap();
        migrate(&connection).unwrap();
        assert_eq!(
            pending(&connection, "owner", "relay").unwrap()[0].id,
            attempt
        );
        assert_eq!(
            connection
                .query_row("SELECT value FROM evidence", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "preserved"
        );
        let broken = Connection::open_in_memory().unwrap();
        broken
            .execute_batch(
                "CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER);
            CREATE TABLE workspace_provider_attempts(wrong TEXT);",
            )
            .unwrap();
        assert!(migrate(&broken).is_err());
        assert_eq!(
            broken
                .query_row("SELECT COUNT(*) FROM archive_migrations", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn channel_workspaces_uncertain_side_effect_blocks_binding_and_retry_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.db");
        let attempt;
        {
            let connection = open_archive_db(&path).unwrap();
            attempt = begin(&connection, "owner", "relay", "agent").unwrap();
            std::fs::write(
                directory.path().join("synthetic-provider-effect"),
                "started",
            )
            .unwrap();
        }
        let connection = open_archive_db(&path).unwrap();
        assert!(require_released(&connection, "relay").is_err());
        let root = directory.path().canonicalize().unwrap();
        let channel = uuid::Uuid::new_v4();
        assert!(crate::archive::channel_workspaces::set(
            &connection,
            "owner",
            "relay",
            channel,
            0,
            root.to_str()
        )
        .is_err());
        assert!(begin(&connection, "owner", "relay", "agent").is_err());
        assert!(pending(&connection, "other", "relay").unwrap().is_empty());
        assert!(begin(&connection, "other", "relay", "agent").is_err());
        assert!(crate::archive::channel_workspaces::set(
            &connection,
            "other",
            "relay",
            channel,
            0,
            root.to_str()
        )
        .is_err());
        assert!(require_released(&connection, "other-relay").is_ok());
        assert!(resolve(&connection, "other", "relay", &attempt).is_err());
        resolve(&connection, "owner", "relay", &attempt).unwrap();
        crate::archive::channel_workspaces::set(
            &connection,
            "owner",
            "relay",
            channel,
            0,
            root.to_str(),
        )
        .unwrap();
        let next = begin(&connection, "owner", "relay", "agent").unwrap();
        assert_ne!(attempt, next);
        assert!(resolve(&connection, "owner", "relay", &attempt).is_err());
        assert!(require_released(&connection, "relay").is_err());
    }
}
