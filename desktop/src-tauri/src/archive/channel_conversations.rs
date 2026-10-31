//! Channels where the owner is heard without tagging anyone.
//!
//! An agent answers when it is `@`-mentioned. That is the right default for a
//! busy project room — one untargeted message there would otherwise cost a
//! turn from every agent in it — but it makes a small room unusable as a
//! conversation: the owner types "good job everyone" and nothing happens,
//! because nobody was addressed by name.
//!
//! A row here marks one channel as a room the owner can simply talk in. The
//! set is passed to each agent at spawn (`BUZZ_ACP_OPEN_CHANNELS`), where the
//! harness lifts the mention requirement **for the owner's own messages only**.
//! That restriction is the reason this is safe: `owner-only` access admits
//! verified same-owner sibling agents as well as the human, so opening a room
//! to everyone would make each agent's reply wake every other agent in it,
//! without end.
//!
//! Presence is the whole record — there is no "off" row, because a channel
//! that is not listed is already mention-only.

use rusqlite::{params, Connection, TransactionBehavior};
use uuid::Uuid;

/// Upper bound on conversation channels per identity.
///
/// Each one widens a relay subscription, so the set is meant to stay small —
/// the rooms the owner actually talks in, not a second copy of the channel
/// list. The limit is a guard against a runaway writer, not a target.
pub(crate) const MAX_CONVERSATION_CHANNELS: usize = 64;

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    let is_applied = |connection: &Connection| -> Result<bool, String> {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name = 'channel_conversations_v1')",
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
                "CREATE TABLE channel_conversations (
                identity_pubkey TEXT NOT NULL,
                relay_url TEXT NOT NULL,
                channel_id TEXT NOT NULL,
                PRIMARY KEY(identity_pubkey, relay_url, channel_id)
            );
            INSERT INTO archive_migrations(name, applied_at)
            VALUES ('channel_conversations_v1', unixepoch());",
            )
            .map_err(|error| format!("channel conversation migration: {error}"))?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

/// Every conversation channel for this identity on this relay.
pub(crate) fn list(conn: &Connection, owner: &str, relay: &str) -> Result<Vec<Uuid>, String> {
    let mut statement = conn
        .prepare(
            "SELECT channel_id FROM channel_conversations
             WHERE identity_pubkey = ?1 AND relay_url = ?2 ORDER BY channel_id LIMIT ?3",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![owner, relay, MAX_CONVERSATION_CHANNELS as i64],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| error.to_string())?;
    let mut channels = Vec::new();
    for row in rows {
        let raw = row.map_err(|error| error.to_string())?;
        // A row that is not a UUID cannot name a channel, so it is skipped
        // rather than failing the read: one bad row must not cost the owner
        // every other room's setting.
        match Uuid::parse_str(&raw) {
            Ok(id) => channels.push(id),
            Err(_) => continue,
        }
    }
    Ok(channels)
}

/// Turn conversation mode on or off for one channel.
///
/// Returns the full set afterwards, so a caller that has to hand it to a
/// spawning agent never has to read it back separately and never acts on a
/// set that changed in between.
pub(crate) fn set(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    enabled: bool,
) -> Result<Vec<Uuid>, String> {
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if enabled {
        let count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM channel_conversations
                 WHERE identity_pubkey = ?1 AND relay_url = ?2 AND channel_id != ?3",
                params![owner, relay, channel.to_string()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if count as usize >= MAX_CONVERSATION_CHANNELS {
            return Err(format!(
                "at most {MAX_CONVERSATION_CHANNELS} conversation channels; turn one off first"
            ));
        }
        transaction
            .execute(
                "INSERT OR IGNORE INTO channel_conversations(identity_pubkey, relay_url, channel_id)
                 VALUES (?1, ?2, ?3)",
                params![owner, relay, channel.to_string()],
            )
            .map_err(|error| error.to_string())?;
    } else {
        transaction
            .execute(
                "DELETE FROM channel_conversations
                 WHERE identity_pubkey = ?1 AND relay_url = ?2 AND channel_id = ?3",
                params![owner, relay, channel.to_string()],
            )
            .map_err(|error| error.to_string())?;
    }
    let channels = list(&transaction, owner, relay)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(channels)
}

#[cfg(test)]
#[path = "channel_conversations_tests.rs"]
mod tests;
