//! Owner approval queue for gated project actions.
//!
//! A harness gate never grants a restricted action on its own: it denies the tool
//! call, records the request here, and the owner decides in the desktop. Approving a
//! check command adds it to the team's approved list (a new team generation), so the
//! agent's next turn runs with the updated authority. Records are device-local and
//! scoped by owner, relay and channel like every other project table.

use buzz_core::local_workspace::ProjectSessionScope;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Upper bound on pending requests per channel; beyond it the gate still denies,
/// but stops recording so a runaway loop cannot fill the database.
pub const MAX_PENDING_PER_CHANNEL: usize = 256;
/// Upper bound on a stored summary (command line or path list).
pub const MAX_SUMMARY_BYTES: usize = 2048;

/// Request categories, matching the gate's classification of the tool call.
pub const CATEGORIES: &[&str] = &[
    "execute", "edit", "read", "fetch", "commit", "publish", "channel", "other",
];

/// One recorded request awaiting or carrying an owner decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Stable identifier.
    pub request_id: String,
    /// Channel whose project the action targeted.
    pub channel_id: Uuid,
    /// Agent that attempted the action.
    pub agent_pubkey: String,
    /// One of [`CATEGORIES`].
    pub category: String,
    /// Command line or path list, bounded.
    pub summary: String,
    /// Why the gate could not allow it.
    pub reason: String,
    /// `pending`, `approved`, or `denied`.
    pub state: String,
    /// Unix seconds.
    pub requested_at: i64,
    /// Unix seconds of the owner decision, if any.
    pub decided_at: Option<i64>,
    /// Optional owner note.
    pub decision_note: Option<String>,
    /// Binding generation at request time.
    pub binding_generation: i64,
    /// Team generation at request time.
    pub team_generation: i64,
}

/// Input for [`record`].
pub struct NewApprovalRequest<'a> {
    /// One of [`CATEGORIES`].
    pub category: &'a str,
    /// Command line or path list.
    pub summary: &'a str,
    /// Gate reason.
    pub reason: &'a str,
}

/// Create the approval table inside the caller's migration transaction.
pub fn migrate(conn: &Connection) -> Result<(), String> {
    let is_applied = |connection: &Connection| -> Result<bool, String> {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name = 'project_approval_requests_v1')",
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
                "CREATE TABLE project_approval_requests (
                    request_id TEXT PRIMARY KEY,
                    identity_pubkey TEXT NOT NULL,
                    relay_url TEXT NOT NULL,
                    channel_id TEXT NOT NULL,
                    agent_pubkey TEXT NOT NULL,
                    category TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    reason TEXT NOT NULL,
                    state TEXT NOT NULL CHECK(state IN ('pending','approved','denied')),
                    requested_at INTEGER NOT NULL,
                    decided_at INTEGER,
                    decision_note TEXT,
                    binding_generation INTEGER NOT NULL,
                    team_generation INTEGER NOT NULL
                );
                CREATE UNIQUE INDEX one_pending_project_approval
                    ON project_approval_requests(identity_pubkey, relay_url, channel_id, agent_pubkey, category, summary)
                    WHERE state = 'pending';
                CREATE INDEX project_approval_requests_scope
                    ON project_approval_requests(identity_pubkey, relay_url, channel_id, state);
                INSERT INTO archive_migrations(name, applied_at)
                VALUES ('project_approval_requests_v1', unixepoch());",
            )
            .map_err(|error| format!("project approval migration: {error}"))?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

fn now() -> Result<i64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| error.to_string())
}

/// Record one denied action. A pending request with the same agent, category and
/// summary is returned instead of duplicated.
pub fn record(
    conn: &Connection,
    scope: &ProjectSessionScope,
    request: NewApprovalRequest<'_>,
) -> Result<String, String> {
    if !CATEGORIES.contains(&request.category) {
        return Err("unsupported approval category".into());
    }
    let summary: String = request
        .summary
        .chars()
        .take(MAX_SUMMARY_BYTES / 4)
        .collect();
    let summary = if summary.len() > MAX_SUMMARY_BYTES {
        summary[..MAX_SUMMARY_BYTES].to_string()
    } else {
        summary
    };
    let channel = scope.channel_id.to_string();
    let existing: Option<String> = conn
        .query_row(
            "SELECT request_id FROM project_approval_requests
             WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3 AND agent_pubkey=?4
               AND category=?5 AND summary=?6 AND state='pending'",
            params![
                scope.owner_pubkey,
                scope.relay_url,
                channel,
                scope.agent_pubkey,
                request.category,
                summary
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let pending: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM project_approval_requests
             WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3 AND state='pending'",
            params![scope.owner_pubkey, scope.relay_url, channel],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if pending as usize >= MAX_PENDING_PER_CHANNEL {
        return Err("pending approval queue is full; decide existing requests first".into());
    }
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO project_approval_requests(request_id, identity_pubkey, relay_url, channel_id,
            agent_pubkey, category, summary, reason, state, requested_at, binding_generation, team_generation)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?10, ?11)",
        params![
            id,
            scope.owner_pubkey,
            scope.relay_url,
            channel,
            scope.agent_pubkey,
            request.category,
            summary,
            request.reason.chars().take(512).collect::<String>(),
            now()?,
            scope.binding_generation,
            scope.team_generation
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(id)
}

fn row_to_request(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApprovalRequest> {
    let channel: String = row.get(1)?;
    Ok(ApprovalRequest {
        request_id: row.get(0)?,
        channel_id: Uuid::parse_str(&channel).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        agent_pubkey: row.get(2)?,
        category: row.get(3)?,
        summary: row.get(4)?,
        reason: row.get(5)?,
        state: row.get(6)?,
        requested_at: row.get(7)?,
        decided_at: row.get(8)?,
        decision_note: row.get(9)?,
        binding_generation: row.get(10)?,
        team_generation: row.get(11)?,
    })
}

const SELECT: &str =
    "SELECT request_id, channel_id, agent_pubkey, category, summary, reason, state,
    requested_at, decided_at, decision_note, binding_generation, team_generation
    FROM project_approval_requests";

/// List requests for one owner and relay, optionally one channel, newest first.
/// Pending requests come first. Bounded to 512 rows.
pub fn list(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Option<Uuid>,
) -> Result<Vec<ApprovalRequest>, String> {
    let sql = format!(
        "{SELECT} WHERE identity_pubkey=?1 AND relay_url=?2 AND (?3 IS NULL OR channel_id=?3)
         ORDER BY CASE state WHEN 'pending' THEN 0 ELSE 1 END, requested_at DESC LIMIT 512"
    );
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![owner, relay, channel.map(|id| id.to_string())],
            row_to_request,
        )
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// Count pending requests for one channel.
pub fn count_pending(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM project_approval_requests
         WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3 AND state='pending'",
        params![owner, relay, channel.to_string()],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

/// The state of one request in this community — `pending`, `approved` or
/// `denied` — or `None` when there is no such request.
///
/// Read by a harness holding the tool call open while the owner decides.
pub fn state(
    conn: &Connection,
    owner: &str,
    relay: &str,
    request_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT state FROM project_approval_requests
         WHERE identity_pubkey=?1 AND relay_url=?2 AND request_id=?3",
        params![owner, relay, request_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// Record the owner's decision on a pending request. Deciding is not execution:
/// an approved check still only runs when the agent retries under the new grant.
pub fn decide(
    conn: &Connection,
    owner: &str,
    relay: &str,
    request_id: &str,
    approve: bool,
    note: Option<&str>,
) -> Result<ApprovalRequest, String> {
    let changed = conn
        .execute(
            "UPDATE project_approval_requests
             SET state=?4, decided_at=?5, decision_note=?6
             WHERE identity_pubkey=?1 AND relay_url=?2 AND request_id=?3 AND state='pending'",
            params![
                owner,
                relay,
                request_id,
                if approve { "approved" } else { "denied" },
                now()?,
                note.map(|note| note.chars().take(512).collect::<String>())
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("approval request is not pending in this community".into());
    }
    conn.query_row(
        &format!("{SELECT} WHERE identity_pubkey=?1 AND relay_url=?2 AND request_id=?3"),
        params![owner, relay, request_id],
        row_to_request,
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(channel: Uuid) -> ProjectSessionScope {
        ProjectSessionScope {
            owner_pubkey: "owner".into(),
            relay_url: "wss://example.test".into(),
            agent_pubkey: "agent".into(),
            channel_id: channel,
            root: "/synthetic/project".into(),
            binding_generation: 2,
            team_generation: 3,
        }
    }

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER NOT NULL);",
        )
        .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        conn
    }

    /// What a harness waiting on the owner sees: pending until decided, the
    /// decision after, and nothing for a request it cannot name.
    #[test]
    fn state_follows_the_owner_decision_within_one_community() {
        let conn = database();
        let channel = Uuid::new_v4();
        let id = record(
            &conn,
            &scope(channel),
            NewApprovalRequest {
                category: "execute",
                summary: "cargo test",
                reason: "not an approved check",
            },
        )
        .unwrap();
        let state_of =
            |owner: &str, id: &str| state(&conn, owner, "wss://example.test", id).unwrap();
        assert_eq!(state_of("owner", &id).as_deref(), Some("pending"));
        assert_eq!(state_of("other-owner", &id), None);
        assert_eq!(state_of("owner", "no-such-request"), None);
        decide(&conn, "owner", "wss://example.test", &id, false, None).unwrap();
        assert_eq!(state_of("owner", &id).as_deref(), Some("denied"));
    }

    #[test]
    fn pending_requests_dedupe_and_decisions_are_scoped() {
        let conn = database();
        let channel = Uuid::new_v4();
        let scope = scope(channel);
        let request = |summary: &'static str| NewApprovalRequest {
            category: "execute",
            summary,
            reason: "not an approved check",
        };
        let first = record(&conn, &scope, request("cargo test")).unwrap();
        let again = record(&conn, &scope, request("cargo test")).unwrap();
        assert_eq!(
            first, again,
            "a pending duplicate must not create a second row"
        );
        let other = record(&conn, &scope, request("pnpm test")).unwrap();
        assert_ne!(first, other);
        assert_eq!(
            count_pending(&conn, "owner", "wss://example.test", channel).unwrap(),
            2
        );
        assert!(list(&conn, "owner", "wss://other.test", None)
            .unwrap()
            .is_empty());
        assert!(decide(
            &conn,
            "other-owner",
            "wss://example.test",
            &first,
            true,
            None
        )
        .is_err());
        let decided = decide(
            &conn,
            "owner",
            "wss://example.test",
            &first,
            true,
            Some("ok"),
        )
        .unwrap();
        assert_eq!(decided.state, "approved");
        assert_eq!(decided.decision_note.as_deref(), Some("ok"));
        assert!(decide(&conn, "owner", "wss://example.test", &first, false, None).is_err());
        let replay = record(&conn, &scope, request("cargo test")).unwrap();
        assert_ne!(
            replay, first,
            "a decided request never absorbs a new attempt"
        );
        assert!(record(
            &conn,
            &scope,
            NewApprovalRequest {
                category: "bogus",
                summary: "x",
                reason: "y"
            }
        )
        .is_err());
        let listed = list(&conn, "owner", "wss://example.test", Some(channel)).unwrap();
        assert_eq!(listed.len(), 3);
        assert!(listed
            .iter()
            .take(2)
            .all(|request| request.state == "pending"));
    }

    #[test]
    fn pending_queue_is_bounded_per_channel() {
        let conn = database();
        let scope = scope(Uuid::new_v4());
        for index in 0..MAX_PENDING_PER_CHANNEL {
            let summary = format!("command-{index}");
            record(
                &conn,
                &scope,
                NewApprovalRequest {
                    category: "execute",
                    summary: &summary,
                    reason: "r",
                },
            )
            .unwrap();
        }
        assert!(record(
            &conn,
            &scope,
            NewApprovalRequest {
                category: "execute",
                summary: "one-too-many",
                reason: "r"
            }
        )
        .is_err());
    }
}
