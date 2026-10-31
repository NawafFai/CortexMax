use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Device-local creation request; only the signed channel event may leave this device.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectChannelRequest {
    /// Stable operation and channel identifier across retries.
    pub request_id: Uuid,
    /// Public channel name.
    pub name: String,
    /// Public channel description.
    pub description: Option<String>,
    /// Native stream or forum kind.
    pub channel_type: String,
    /// Native open or private visibility.
    pub visibility: String,
    /// Canonical device-local folder, never included in the event.
    pub root: String,
}

pub(crate) struct Creation {
    pub request: ProjectChannelRequest,
    pub event_json: String,
    pub result_json: Option<String>,
    pub cancelled: bool,
}

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    let applied = |conn: &Connection| -> Result<bool, String> {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name = 'project_channel_creations_v1')", [], |row| row.get(0))
            .map_err(|error| error.to_string())
    };
    if applied(conn)? {
        return migrate_abandonment(conn);
    }
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !applied(&transaction)? {
        transaction.execute_batch("CREATE TABLE project_channel_creations (
            identity_pubkey TEXT NOT NULL, relay_url TEXT NOT NULL, request_id TEXT NOT NULL,
            request_json TEXT NOT NULL, event_json TEXT NOT NULL, result_json TEXT,
            PRIMARY KEY(identity_pubkey, relay_url, request_id)
        ); CREATE UNIQUE INDEX one_pending_project_channel ON project_channel_creations(identity_pubkey, relay_url) WHERE result_json IS NULL;
        INSERT INTO archive_migrations(name, applied_at) VALUES ('project_channel_creations_v1', unixepoch());")
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    migrate_abandonment(conn)
}

fn migrate_abandonment(conn: &Connection) -> Result<(), String> {
    let applied = |conn: &Connection| -> Result<bool, String> {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name = 'project_channel_creations_v2')", [], |row| row.get(0))
            .map_err(|error| error.to_string())
    };
    if applied(conn)? {
        return Ok(());
    }
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !applied(&transaction)? {
        transaction.execute_batch("ALTER TABLE project_channel_creations ADD COLUMN cancelled INTEGER NOT NULL DEFAULT 0;
            DROP INDEX one_pending_project_channel;
            CREATE UNIQUE INDEX one_pending_project_channel ON project_channel_creations(identity_pubkey, relay_url) WHERE result_json IS NULL AND cancelled=0;
            INSERT INTO archive_migrations(name, applied_at) VALUES ('project_channel_creations_v2', unixepoch());")
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(crate) fn load(
    conn: &Connection,
    owner: &str,
    relay: &str,
    request: Uuid,
) -> Result<Option<Creation>, String> {
    let row: Option<(String, String, Option<String>, bool)> = conn.query_row(
        "SELECT request_json, event_json, result_json, cancelled FROM project_channel_creations WHERE identity_pubkey=?1 AND relay_url=?2 AND request_id=?3",
        params![owner, relay, request.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    ).optional().map_err(|error| error.to_string())?;
    row.map(|(request, event_json, result_json, cancelled)| {
        Ok(Creation {
            request: serde_json::from_str(&request)
                .map_err(|error| format!("invalid saved creation: {error}"))?,
            event_json,
            result_json,
            cancelled,
        })
    })
    .transpose()
}

pub(crate) fn pending(
    conn: &Connection,
    owner: &str,
    relay: &str,
) -> Result<Option<ProjectChannelRequest>, String> {
    let raw: Option<String> = conn.query_row(
        "SELECT request_json FROM project_channel_creations WHERE identity_pubkey=?1 AND relay_url=?2 AND result_json IS NULL AND cancelled=0",
        params![owner, relay], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|error| format!("invalid saved creation: {error}"))
    })
    .transpose()
}

pub(crate) fn require_settled(conn: &Connection, owner: &str, relay: &str) -> Result<(), String> {
    if pending(conn, owner, relay)?.is_some() {
        return Err("channel creation outcome is pending; reopen Create channel and resume the saved request before changing project bindings".into());
    }
    Ok(())
}

pub(crate) fn prepare(
    conn: &Connection,
    owner: &str,
    relay: &str,
    request: &ProjectChannelRequest,
    event: &str,
) -> Result<Creation, String> {
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if let Some(saved) = load(&transaction, owner, relay, request.request_id)? {
        if saved.cancelled {
            return Err("this local creation was abandoned; no event will be retried".into());
        }
        if saved.request != *request {
            return Err("saved channel creation differs; resume its original request".into());
        }
        return Ok(saved);
    }
    require_settled(&transaction, owner, relay)?;
    let count: i64 = transaction.query_row("SELECT COUNT(*) FROM project_channel_creations WHERE identity_pubkey=?1 AND relay_url=?2", params![owner, relay], |row| row.get(0)).map_err(|error| error.to_string())?;
    if count >= 256 {
        return Err("project channel creation limit reached".into());
    }
    let root = super::channel_workspaces::validate_root(&request.root)?;
    super::channel_workspaces::set_in_transaction(
        &transaction,
        owner,
        relay,
        request.request_id,
        0,
        Some(root),
    )?;
    let request_json = serde_json::to_string(request).map_err(|error| error.to_string())?;
    transaction.execute("INSERT INTO project_channel_creations(identity_pubkey, relay_url, request_id, request_json, event_json) VALUES (?1,?2,?3,?4,?5)", params![owner, relay, request.request_id.to_string(), request_json, event]).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(Creation {
        request: request.clone(),
        event_json: event.to_owned(),
        result_json: None,
        cancelled: false,
    })
}

pub(crate) fn complete(
    conn: &Connection,
    owner: &str,
    relay: &str,
    request: Uuid,
    result: &str,
) -> Result<(), String> {
    let changed = conn.execute("UPDATE project_channel_creations SET result_json=?4 WHERE identity_pubkey=?1 AND relay_url=?2 AND request_id=?3 AND result_json IS NULL AND cancelled=0", params![owner, relay, request.to_string(), result]).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("creation changed before its receipt was saved; reload".into());
    }
    Ok(())
}

pub(crate) fn abandon(
    conn: &Connection,
    owner: &str,
    relay: &str,
    request_id: Uuid,
) -> Result<(), String> {
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let saved = load(&transaction, owner, relay, request_id)?
        .ok_or("saved creation not found in this scope")?;
    if saved.cancelled || saved.result_json.is_some() {
        return Err("creation is already reconciled; reload".into());
    }
    super::channel_workspaces::set_in_transaction(&transaction, owner, relay, request_id, 1, None)?;
    transaction.execute("UPDATE project_channel_creations SET cancelled=1 WHERE identity_pubkey=?1 AND relay_url=?2 AND request_id=?3", params![owner, relay, request_id.to_string()]).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{channel_workspaces, store::open_archive_db};

    fn request(root: &std::path::Path) -> ProjectChannelRequest {
        ProjectChannelRequest {
            request_id: Uuid::new_v4(),
            name: "synthetic".into(),
            description: None,
            channel_type: "stream".into(),
            visibility: "private".into(),
            root: root.to_str().unwrap().into(),
        }
    }

    #[test]
    fn channel_workspaces_creation_retries_exact_event_after_restart_and_lost_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("archive.db");
        let input = request(&root);
        {
            let conn = open_archive_db(&path).unwrap();
            prepare(&conn, "owner-a", "relay", &input, "original signed event").unwrap();
            std::fs::write(
                root.join("synthetic-relay-effect"),
                input.request_id.to_string(),
            )
            .unwrap();
        }
        let conn = open_archive_db(&path).unwrap();
        let saved = prepare(
            &conn,
            "owner-a",
            "relay",
            &input,
            "must not republish this event",
        )
        .unwrap();
        assert_eq!(saved.event_json, "original signed event");
        assert!(saved.result_json.is_none());
        assert_eq!(
            channel_workspaces::list(&conn, "owner-a", "relay").unwrap()[0].generation,
            1
        );
        assert_eq!(
            pending(&conn, "owner-a", "relay").unwrap(),
            Some(input.clone())
        );
        assert!(pending(&conn, "owner-b", "relay").unwrap().is_none());
        assert!(pending(&conn, "owner-a", "other-relay").unwrap().is_none());
        assert!(
            channel_workspaces::set(&conn, "owner-a", "relay", input.request_id, 1, None).is_err()
        );
        let mut changed = input.clone();
        changed.name = "different".into();
        assert!(prepare(&conn, "owner-a", "relay", &changed, "different").is_err());
        assert!(prepare(&conn, "owner-a", "relay", &request(&root), "different").is_err());
        assert!(complete(&conn, "owner-b", "relay", input.request_id, "receipt").is_err());
        complete(&conn, "owner-a", "relay", input.request_id, "receipt").unwrap();
        assert!(pending(&conn, "owner-a", "relay").unwrap().is_none());
        let completed = prepare(&conn, "owner-a", "relay", &input, "different").unwrap();
        assert_eq!(completed.result_json.as_deref(), Some("receipt"));
        channel_workspaces::set(&conn, "owner-a", "relay", input.request_id, 1, None).unwrap();
        assert!(channel_workspaces::list(&conn, "owner-b", "relay")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn channel_workspaces_creation_is_atomic_and_keeps_projects_separate() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let conn = open_archive_db(&root.join("archive.db")).unwrap();
        conn.execute_batch("CREATE TRIGGER simulate_disk_failure BEFORE INSERT ON project_channel_creations BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
        let input = request(&first);
        assert!(prepare(&conn, "owner-a", "relay", &input, "event-a").is_err());
        assert!(channel_workspaces::list(&conn, "owner-a", "relay")
            .unwrap()
            .is_empty());
        assert!(pending(&conn, "owner-a", "relay").unwrap().is_none());
        conn.execute_batch("DROP TRIGGER simulate_disk_failure")
            .unwrap();
        prepare(&conn, "owner-a", "relay", &input, "event-a").unwrap();
        let other = request(&second);
        prepare(&conn, "owner-b", "relay", &other, "event-b").unwrap();
        assert_eq!(
            channel_workspaces::list(&conn, "owner-a", "relay").unwrap()[0]
                .root
                .as_deref(),
            first.to_str()
        );
        assert_eq!(
            channel_workspaces::list(&conn, "owner-b", "relay").unwrap()[0]
                .root
                .as_deref(),
            second.to_str()
        );
    }

    #[test]
    fn channel_workspaces_creation_migration_is_atomic_and_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER); CREATE TABLE evidence(value TEXT); INSERT INTO evidence VALUES ('preserved');").unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            conn.query_row("SELECT value FROM evidence", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            "preserved"
        );
        let broken = Connection::open_in_memory().unwrap();
        broken.execute_batch("CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER); CREATE TABLE project_channel_creations(wrong TEXT);").unwrap();
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
    fn channel_workspaces_abandon_preserves_evidence_and_fences_old_retries() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("archive.db");
        let input = request(&root);
        let conn = open_archive_db(&path).unwrap();
        prepare(&conn, "owner", "relay", &input, "original signed event").unwrap();
        assert!(abandon(&conn, "other", "relay", input.request_id).is_err());
        abandon(&conn, "owner", "relay", input.request_id).unwrap();
        drop(conn);
        let conn = open_archive_db(&path).unwrap();
        assert!(pending(&conn, "owner", "relay").unwrap().is_none());
        let saved = load(&conn, "owner", "relay", input.request_id)
            .unwrap()
            .unwrap();
        assert!(saved.cancelled);
        assert_eq!(saved.event_json, "original signed event");
        let binding = channel_workspaces::list(&conn, "owner", "relay")
            .unwrap()
            .remove(0);
        assert_eq!(binding.generation, 2);
        assert!(binding.root.is_none());
        assert!(prepare(&conn, "owner", "relay", &input, "must not retry").is_err());
        assert!(complete(&conn, "owner", "relay", input.request_id, "late receipt").is_err());
        assert!(root.is_dir());
    }

    #[test]
    fn channel_workspaces_creation_upgrade_preserves_pending_v1_event() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER);
            INSERT INTO archive_migrations VALUES('project_channel_creations_v1', 1);
            CREATE TABLE project_channel_creations(identity_pubkey TEXT NOT NULL, relay_url TEXT NOT NULL, request_id TEXT NOT NULL,
                request_json TEXT NOT NULL, event_json TEXT NOT NULL, result_json TEXT, PRIMARY KEY(identity_pubkey, relay_url, request_id));
            CREATE UNIQUE INDEX one_pending_project_channel ON project_channel_creations(identity_pubkey, relay_url) WHERE result_json IS NULL;").unwrap();
        let input = request(std::path::Path::new("/synthetic/project"));
        conn.execute("INSERT INTO project_channel_creations(identity_pubkey, relay_url, request_id, request_json, event_json) VALUES('owner','relay',?1,?2,'original signed event')",
            params![input.request_id.to_string(), serde_json::to_string(&input).unwrap()]).unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            pending(&conn, "owner", "relay").unwrap(),
            Some(input.clone())
        );
        let saved = load(&conn, "owner", "relay", input.request_id)
            .unwrap()
            .unwrap();
        assert!(!saved.cancelled);
        assert_eq!(saved.event_json, "original signed event");
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM archive_migrations WHERE name='project_channel_creations_v2'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}
