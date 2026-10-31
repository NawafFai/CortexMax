//! Native durable receipts for bounded edits; uncertain side effects are never retried automatically.

use super::{files, ProjectPolicy};
use buzz_core::local_workspace::ProjectSessionScope;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;

/// Apply the native archive migration; only the desktop archive initializer calls this.
pub fn migrate(conn: &Connection) -> Result<(), String> {
    let applied = |conn: &Connection| {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name='project_file_operations_v1')",[],|row|row.get::<_,bool>(0)).map_err(|error|error.to_string())
    };
    if applied(conn)? {
        return Ok(());
    }
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !applied(&transaction)? {
        transaction.execute_batch("CREATE TABLE project_file_operations (
            operation_id TEXT PRIMARY KEY,identity_pubkey TEXT NOT NULL,relay_url TEXT NOT NULL,channel_id TEXT NOT NULL,
            scope TEXT NOT NULL,relative_path TEXT NOT NULL,before_sha256 TEXT NOT NULL,after_sha256 TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('pending','completed','resolved')),receipt TEXT,created_at INTEGER NOT NULL);
            CREATE INDEX project_file_operations_pending ON project_file_operations(relay_url,state);
            INSERT INTO archive_migrations(name,applied_at) VALUES('project_file_operations_v1',unixepoch());")
            .map_err(|error|error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

/// Fail closed while any owner on this community has an unresolved local file side effect.
pub fn require_reconciled(conn: &Connection, relay: &str) -> Result<(), String> {
    let pending:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM project_file_operations WHERE relay_url=?1 AND state='pending')",[relay],|row|row.get(0)).map_err(|error|error.to_string())?;
    if pending {
        return Err("a native project edit has an uncertain outcome; inspect its receipt and reconcile it before new edits or binding changes".into());
    }
    Ok(())
}

/// Fail closed while this channel's project has an unresolved local file side effect.
/// Other channels' pending receipts do not block this channel; they stay visible in
/// their own recovery tab.
pub fn require_reconciled_for_channel(
    conn: &Connection,
    relay: &str,
    channel: &str,
) -> Result<(), String> {
    let pending:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM project_file_operations WHERE relay_url=?1 AND channel_id=?2 AND state='pending')",[relay,channel],|row|row.get(0)).map_err(|error|error.to_string())?;
    if pending {
        return Err("this project has a native edit with an uncertain outcome; inspect its receipt and reconcile it before new edits, grants, or binding changes".into());
    }
    Ok(())
}

fn check(
    policy: &ProjectPolicy,
    conn: &Connection,
    scope: &ProjectSessionScope,
) -> Result<(), String> {
    for capability in ["read", "edit"] {
        let current = policy
            .resolve(conn, &scope.channel_id.to_string(), capability)?
            .ok_or("project detached")?;
        if current != *scope {
            return Err("project session authority changed before edit".into());
        }
    }
    Ok(())
}

pub(super) fn replace(
    policy: &ProjectPolicy,
    scope: &ProjectSessionScope,
    operation_id: &str,
    path: &str,
    before: &str,
    content: &str,
    operation: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    files::validate_relative(path)?;
    if operation_id.len() > 256
        || (before != "absent"
            && (before.len() != 64 || !before.bytes().all(|byte| byte.is_ascii_hexdigit())))
        || content.len() > 65536
    {
        return Err("invalid bounded project edit".into());
    }
    let after = files::hash(content);
    let scope_json = serde_json::to_string(scope).map_err(|error| error.to_string())?;
    let mut conn = policy.open()?;
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    check(policy, &transaction, scope)?;
    let existing:Option<(String,String,String,String,String,Option<String>)>=transaction.query_row(
        "SELECT scope,relative_path,before_sha256,after_sha256,state,receipt FROM project_file_operations WHERE operation_id=?1",[operation_id],
        |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional().map_err(|error|error.to_string())?;
    if let Some((prior_scope, prior_path, prior_before, prior_after, state, receipt)) = existing {
        if prior_scope != scope_json
            || prior_path != path
            || prior_before != before
            || prior_after != after
        {
            return Err("operation identity reused for different work".into());
        }
        if state == "completed" {
            return receipt.ok_or("completed edit receipt is missing".into());
        }
        return Err(
            "edit outcome is uncertain or owner-resolved; do not replay the operation".into(),
        );
    }
    require_reconciled(&transaction, &scope.relay_url)?;
    let count:i64=transaction.query_row("SELECT COUNT(*) FROM project_file_operations WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3",params![scope.owner_pubkey,scope.relay_url,scope.channel_id.to_string()],|row|row.get(0)).map_err(|error|error.to_string())?;
    if count >= 10000 {
        return Err("project edit receipt limit reached; preserve audit history".into());
    }
    transaction.execute("INSERT INTO project_file_operations(operation_id,identity_pubkey,relay_url,channel_id,scope,relative_path,before_sha256,after_sha256,state,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending',unixepoch())",
        params![operation_id,scope.owner_pubkey,scope.relay_url,scope.channel_id.to_string(),scope_json,path,before,after]).map_err(|error|error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("edit journal is pending; authority lock failed: {error}"))?;
    check(policy, &transaction, scope)?;
    let result=operation().map_err(|error|format!("edit receipt {operation_id} remains pending; inspect the file before retrying: {error}"))?;
    transaction.execute("UPDATE project_file_operations SET state='completed',receipt=?2 WHERE operation_id=?1 AND state='pending'",params![operation_id,result]).map_err(|error|error.to_string())?;
    transaction.commit().map_err(|error| {
        format!("file may have changed but its receipt did not commit: {error}")
    })?;
    Ok(result)
}

/// Owner-visible durable evidence; contains no file contents or identity secrets.
#[derive(Serialize)]
pub struct PendingEdit {
    /// Stable operation identifier, retained after recovery.
    pub operation_id: String,
    /// Original channel of the edit.
    pub channel_id: String,
    /// Relative project path.
    pub relative_path: String,
    /// Content hash expected before the edit.
    pub before_sha256: String,
    /// Intended content hash after the edit.
    pub after_sha256: String,
}

/// Read this owner's pending edits, without inferring completion from model prose.
pub fn pending(conn: &Connection, owner: &str, relay: &str) -> Result<Vec<PendingEdit>, String> {
    let mut statement=conn.prepare("SELECT operation_id,channel_id,relative_path,before_sha256,after_sha256 FROM project_file_operations WHERE identity_pubkey=?1 AND relay_url=?2 AND state='pending' ORDER BY created_at LIMIT 256").map_err(|error|error.to_string())?;
    let rows = statement
        .query_map(params![owner, relay], |row| {
            Ok(PendingEdit {
                operation_id: row.get(0)?,
                channel_id: row.get(1)?,
                relative_path: row.get(2)?,
                before_sha256: row.get(3)?,
                after_sha256: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// Record explicit owner reconciliation against the observed current file hash; never rewrite files.
pub fn reconcile(
    conn: &Connection,
    owner: &str,
    relay: &str,
    operation_id: &str,
    expected_current_sha256: &str,
) -> Result<(), String> {
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let (raw,path):(String,String)=transaction.query_row("SELECT scope,relative_path FROM project_file_operations WHERE operation_id=?1 AND identity_pubkey=?2 AND relay_url=?3 AND state='pending'",params![operation_id,owner,relay],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|_|"pending edit not found in current owner scope")?;
    let scope: ProjectSessionScope =
        serde_json::from_str(&raw).map_err(|_| "invalid pending edit scope")?;
    let matches = if expected_current_sha256 == "absent" {
        files::absent(&scope.root, &path)?
    } else {
        let current: serde_json::Value = serde_json::from_str(&files::read(&scope.root, &path)?)
            .map_err(|_| "invalid current file evidence")?;
        current["sha256"].as_str() == Some(expected_current_sha256)
    };
    if !matches {
        return Err(
            "current file differs from the owner's inspected state; nothing was reconciled".into(),
        );
    }
    let receipt=serde_json::json!({"state":"owner_reconciled_not_reviewed","current_sha256":expected_current_sha256}).to_string();
    transaction
        .execute(
            "UPDATE project_file_operations SET state='resolved',receipt=?2 WHERE operation_id=?1",
            params![operation_id, receipt],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_project_edit_crash_receipt_survives_restart_and_forbids_replay() {
        let directory = tempfile::Builder::new()
            .prefix("buzz-journal-")
            .tempdir()
            .unwrap();
        let base = directory.path().canonicalize().unwrap();
        let root = base.join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("work.txt"), "before").unwrap();
        let database = base.join("archive.db");
        std::fs::write(base.join("agents.json"), "[]").unwrap();
        let conn = Connection::open(&database).unwrap();
        conn.execute_batch("CREATE TABLE archive_migrations(name TEXT PRIMARY KEY,applied_at INTEGER);
            CREATE TABLE channel_workspaces(identity_pubkey TEXT,relay_url TEXT,channel_id TEXT,generation INTEGER,root TEXT);
            CREATE TABLE channel_project_teams(identity_pubkey TEXT,relay_url TEXT,channel_id TEXT,snapshot TEXT);").unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let channel = uuid::Uuid::new_v4();
        conn.execute(
            "INSERT INTO channel_workspaces VALUES('owner','relay',?1,1,?2)",
            params![channel.to_string(), root.to_str()],
        )
        .unwrap();
        conn.execute("INSERT INTO channel_project_teams VALUES('owner','relay',?1,?2)",params![channel.to_string(),serde_json::json!({"generation":1,"binding_generation":1,"paused":false,"coordinator_pubkey":"agent","members":[{"pubkey":"agent","capabilities":["read","edit"]}]}).to_string()]).unwrap();
        drop(conn);
        let policy = ProjectPolicy {
            database: database.to_str().unwrap().into(),
            owner_pubkey: "owner".into(),
            relay_url: "relay".into(),
            agent_pubkey: "agent".into(),
            protected_roots: vec![],
            agent_store: base.join("agents.json").to_str().unwrap().into(),
        };
        let scope = policy.session_scope(&channel.to_string()).unwrap().unwrap();
        let before = files::hash("before");
        assert!(replace(
            &policy,
            &scope,
            "crash-op",
            "work.txt",
            &before,
            "after",
            || {
                files::replace(&scope.root, "work.txt", &before, "after")?;
                Err("synthetic crash after file write before receipt".into())
            }
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("work.txt")).unwrap(),
            "after"
        );
        let called = std::cell::Cell::new(false);
        assert!(replace(
            &policy,
            &scope,
            "crash-op",
            "work.txt",
            &before,
            "after",
            || {
                called.set(true);
                Ok("incorrect replay".into())
            }
        )
        .is_err());
        assert!(!called.get());
        let conn = Connection::open(&database).unwrap();
        assert_eq!(pending(&conn, "owner", "relay").unwrap().len(), 1);
        assert!(reconcile(&conn, "other", "relay", "crash-op", &files::hash("after")).is_err());
        reconcile(&conn, "owner", "relay", "crash-op", &files::hash("after")).unwrap();
        assert!(pending(&conn, "owner", "relay").unwrap().is_empty());
        assert!(replace(
            &policy,
            &scope,
            "crash-op",
            "work.txt",
            &before,
            "after",
            || {
                called.set(true);
                Ok("incorrect replay".into())
            }
        )
        .is_err());
        assert!(!called.get());
        assert!(replace(
            &policy,
            &scope,
            "absent-op",
            "missing/new.txt",
            "absent",
            "new",
            || files::create(&scope.root, "missing/new.txt", "new")
        )
        .is_err());
        reconcile(&conn, "owner", "relay", "absent-op", "absent").unwrap();
        assert!(!root.join("missing").exists());
        assert!(replace(
            &policy,
            &scope,
            "absent-op",
            "missing/new.txt",
            "absent",
            "new",
            || {
                called.set(true);
                Ok("incorrect create replay".into())
            }
        )
        .is_err());
        assert!(!called.get());
    }

    #[test]
    fn native_project_edit_migration_collision_is_not_success() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE archive_migrations(name TEXT PRIMARY KEY,applied_at INTEGER);CREATE TABLE project_file_operations(wrong TEXT);").unwrap();
        assert!(migrate(&conn).is_err());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM archive_migrations", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
