use super::*;
use crate::archive::store::open_archive_db;

#[test]
fn channel_workspaces_persist_scope_and_generation_across_restart() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let database = root.join("archive.db");
    let channel = Uuid::new_v4();
    {
        let conn = open_archive_db(&database).unwrap();
        let binding = set(&conn, "owner", "relay", channel, 0, root.to_str()).unwrap();
        assert_eq!(binding.generation, 1);
        assert!(list(&conn, "other", "relay").unwrap().is_empty());
        assert!(list(&conn, "owner", "other").unwrap().is_empty());
        assert!(set(&conn, "owner", "relay", channel, 0, None).is_err());
        assert_eq!(list(&conn, "owner", "relay").unwrap(), vec![binding]);
    }
    let conn = open_archive_db(&database).unwrap();
    let detached = set(&conn, "owner", "relay", channel, 1, None).unwrap();
    assert_eq!(detached.generation, 2);
    assert!(detached.root.is_none());
    assert!(set(&conn, "owner", "relay", channel, 1, root.to_str()).is_err());
    assert_eq!(
        set(&conn, "owner", "relay", channel, 2, root.to_str())
            .unwrap()
            .generation,
        3
    );
    assert_eq!(list(&conn, "owner", "relay").unwrap().len(), 1);
}

#[test]
fn channel_workspaces_upgrade_preserves_archive_and_rejects_schema_collision() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER);
        CREATE TABLE evidence(value TEXT); INSERT INTO evidence VALUES ('preserved');",
    )
    .unwrap();
    migrate(&conn).unwrap();
    migrate(&conn).unwrap();
    assert_eq!(
        conn.query_row("SELECT value FROM evidence", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "preserved"
    );
    let broken = Connection::open_in_memory().unwrap();
    broken
        .execute_batch(
            "CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER);
        CREATE TABLE channel_workspaces(wrong TEXT);",
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
fn channel_workspaces_reject_missing_relative_and_traversal_paths_without_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let conn = open_archive_db(&root.join("archive.db")).unwrap();
    let channel = Uuid::new_v4();
    for invalid in [
        "relative".to_string(),
        root.join("missing").display().to_string(),
        root.join("../escape").display().to_string(),
    ] {
        assert!(set(&conn, "owner", "relay", channel, 0, Some(&invalid)).is_err());
        assert!(list(&conn, "owner", "relay").unwrap().is_empty());
    }
}

#[cfg(unix)]
#[test]
fn channel_workspaces_reject_symlinks_and_replaced_roots() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let actual = root.join("actual");
    std::fs::create_dir(&actual).unwrap();
    let link = root.join("linked");
    std::os::unix::fs::symlink(&actual, &link).unwrap();
    assert!(validate_root(link.to_str().unwrap()).is_err());
    assert!(validate_root(actual.to_str().unwrap()).is_ok());
    std::fs::rename(&actual, root.join("moved")).unwrap();
    std::os::unix::fs::symlink(root.join("moved"), &actual).unwrap();
    assert!(validate_root(actual.to_str().unwrap()).is_err());
}

#[test]
fn channel_workspaces_concurrent_stale_save_cannot_replace_binding() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let database = root.join("archive.db");
    open_archive_db(&database).unwrap();
    let channel = Uuid::new_v4();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|threads| {
        let save = || {
            let connection = open_archive_db(&database).unwrap();
            barrier.wait();
            set(&connection, "owner", "relay", channel, 0, root.to_str())
        };
        let first = threads.spawn(save);
        let second = threads.spawn(save);
        assert_ne!(
            first.join().unwrap().is_ok(),
            second.join().unwrap().is_ok()
        );
    });
    let connection = open_archive_db(&database).unwrap();
    assert_eq!(
        list(&connection, "owner", "relay").unwrap()[0].generation,
        1
    );
}
