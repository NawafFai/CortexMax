//! Persisted evidence for the project readiness check.
//!
//! The isolated inspection (`inspect_channel_project`) is the only step that
//! proves the actual working directory, branch and revision. Its evidence is
//! stored here keyed by the binding and team generations it was taken under, so
//! readiness can show *when* access was last verified and invalidate it the
//! moment either generation moves. Evidence never becomes a "ready" badge on its
//! own; it is one check among the others.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One stored inspection.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredInspection {
    /// Binding generation the inspection ran under.
    pub binding_generation: i64,
    /// Team generation the inspection ran under.
    pub team_generation: i64,
    /// Unix seconds.
    pub checked_at: i64,
    /// Actual working directory reported by the restricted checker.
    pub actual_cwd: String,
    /// Git branch, if the root is a repository.
    pub branch: Option<String>,
    /// Git revision, if known.
    pub revision: Option<String>,
    /// Uncommitted changes present, if known.
    pub dirty: Option<bool>,
    /// Checker identity string.
    pub runtime: String,
}

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    let is_applied = |connection: &Connection| -> Result<bool, String> {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name = 'channel_project_inspections_v1')",
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
                "CREATE TABLE channel_project_inspections (
                    identity_pubkey TEXT NOT NULL,
                    relay_url TEXT NOT NULL,
                    channel_id TEXT NOT NULL,
                    binding_generation INTEGER NOT NULL,
                    team_generation INTEGER NOT NULL,
                    checked_at INTEGER NOT NULL,
                    evidence TEXT NOT NULL,
                    PRIMARY KEY(identity_pubkey, relay_url, channel_id)
                );
                INSERT INTO archive_migrations(name, applied_at)
                VALUES ('channel_project_inspections_v1', unixepoch());",
            )
            .map_err(|error| format!("project inspection migration: {error}"))?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

/// Store (replace) the latest inspection for a channel.
pub(crate) fn store(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    inspection: &StoredInspection,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO channel_project_inspections(identity_pubkey, relay_url, channel_id,
            binding_generation, team_generation, checked_at, evidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(identity_pubkey, relay_url, channel_id) DO UPDATE SET
            binding_generation = excluded.binding_generation,
            team_generation = excluded.team_generation,
            checked_at = excluded.checked_at,
            evidence = excluded.evidence",
        params![
            owner,
            relay,
            channel.to_string(),
            inspection.binding_generation,
            inspection.team_generation,
            inspection.checked_at,
            serde_json::to_string(inspection).map_err(|error| error.to_string())?
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Load the latest inspection for a channel, if any.
pub(crate) fn load(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
) -> Result<Option<StoredInspection>, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT evidence FROM channel_project_inspections
             WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3",
            params![owner, relay, channel.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(|error| error.to_string()))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_evidence_round_trips_and_replaces() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER NOT NULL);",
        )
        .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let channel = Uuid::new_v4();
        assert!(load(&conn, "o", "r", channel).unwrap().is_none());
        let first = StoredInspection {
            binding_generation: 1,
            team_generation: 1,
            checked_at: 10,
            actual_cwd: "/p".into(),
            branch: Some("main".into()),
            revision: Some("abc".into()),
            dirty: Some(true),
            runtime: "checker".into(),
        };
        store(&conn, "o", "r", channel, &first).unwrap();
        assert_eq!(load(&conn, "o", "r", channel).unwrap(), Some(first.clone()));
        let second = StoredInspection {
            binding_generation: 2,
            checked_at: 20,
            ..first
        };
        store(&conn, "o", "r", channel, &second).unwrap();
        assert_eq!(load(&conn, "o", "r", channel).unwrap(), Some(second));
        assert!(load(&conn, "other", "r", channel).unwrap().is_none());
    }
}
