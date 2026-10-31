//! Handoff records for channel projects.
//!
//! A handoff is a *historical checkpoint*: the text of a handoff file found in
//! the project (or a summary the owner pasted from another provider's session),
//! captured together with the Git revision and binding generation it was taken
//! under. It is never treated as the live state, and it carries the owner's
//! explicit acknowledgement that the previous writer released the project; until
//! that acknowledgement exists, readiness stays blocked.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Upper bound on stored handoff text.
pub const MAX_HANDOFF_BYTES: usize = 64 * 1024;

/// One stored handoff.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffRecord {
    /// `file` (read from the project) or `imported` (owner-supplied summary).
    pub source: String,
    /// Relative file path, or the owner's label for an import.
    pub label: String,
    /// Bounded text.
    pub text: String,
    /// Unix seconds captured.
    pub captured_at: i64,
    /// Binding generation at capture.
    pub binding_generation: i64,
    /// Git revision at capture, if known.
    pub git_revision: Option<String>,
    /// Whether the working tree was dirty at capture, if known.
    pub dirty_at_capture: Option<bool>,
    /// Unix seconds the owner confirmed the previous writer released the project.
    pub previous_writer_released_at: Option<i64>,
}

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    let is_applied = |connection: &Connection| -> Result<bool, String> {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name = 'channel_project_handoffs_v1')",
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
                "CREATE TABLE channel_project_handoffs (
                    identity_pubkey TEXT NOT NULL,
                    relay_url TEXT NOT NULL,
                    channel_id TEXT NOT NULL,
                    record TEXT NOT NULL,
                    PRIMARY KEY(identity_pubkey, relay_url, channel_id)
                );
                INSERT INTO archive_migrations(name, applied_at)
                VALUES ('channel_project_handoffs_v1', unixepoch());",
            )
            .map_err(|error| format!("project handoff migration: {error}"))?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

/// Store (replace) the channel's handoff record. A new capture resets the
/// previous-writer acknowledgement: it describes a different checkpoint.
pub(crate) fn store(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    record: &HandoffRecord,
) -> Result<(), String> {
    if record.text.len() > MAX_HANDOFF_BYTES {
        return Err("handoff text exceeds the stored bound".into());
    }
    conn.execute(
        "INSERT INTO channel_project_handoffs(identity_pubkey, relay_url, channel_id, record)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(identity_pubkey, relay_url, channel_id) DO UPDATE SET record = excluded.record",
        params![
            owner,
            relay,
            channel.to_string(),
            serde_json::to_string(record).map_err(|error| error.to_string())?
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Load the channel's handoff record, if any.
pub(crate) fn load(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
) -> Result<Option<HandoffRecord>, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT record FROM channel_project_handoffs
             WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3",
            params![owner, relay, channel.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(|error| error.to_string()))
        .transpose()
}

/// Record the owner's confirmation that the previous writer released the project.
pub(crate) fn acknowledge_release(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    at: i64,
) -> Result<HandoffRecord, String> {
    let mut record = load(conn, owner, relay, channel)?
        .ok_or("no handoff is recorded for this project; inspect or import one first")?;
    record.previous_writer_released_at = Some(at);
    store(conn, owner, relay, channel, &record)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_records_round_trip_and_new_capture_resets_acknowledgement() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER NOT NULL);",
        )
        .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let channel = Uuid::new_v4();
        assert!(load(&conn, "o", "r", channel).unwrap().is_none());
        assert!(acknowledge_release(&conn, "o", "r", channel, 5).is_err());
        let record = HandoffRecord {
            source: "file".into(),
            label: "HANDOFF.md".into(),
            text: "- [ ] finish tests".into(),
            captured_at: 1,
            binding_generation: 1,
            git_revision: Some("abc".into()),
            dirty_at_capture: Some(true),
            previous_writer_released_at: None,
        };
        store(&conn, "o", "r", channel, &record).unwrap();
        let acknowledged = acknowledge_release(&conn, "o", "r", channel, 9).unwrap();
        assert_eq!(acknowledged.previous_writer_released_at, Some(9));
        let fresh = HandoffRecord {
            captured_at: 2,
            ..record
        };
        store(&conn, "o", "r", channel, &fresh).unwrap();
        assert_eq!(
            load(&conn, "o", "r", channel)
                .unwrap()
                .unwrap()
                .previous_writer_released_at,
            None
        );
        assert!(store(
            &conn,
            "o",
            "r",
            channel,
            &HandoffRecord {
                text: "x".repeat(MAX_HANDOFF_BYTES + 1),
                ..fresh
            }
        )
        .is_err());
    }
}
