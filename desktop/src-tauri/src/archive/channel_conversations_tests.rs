use super::*;
use crate::archive::store::open_archive_db;

/// The setting has to outlive the app, because it is read at agent spawn —
/// which is exactly when the app has just started.
#[test]
fn conversation_mode_survives_a_restart_and_stays_within_its_scope() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("archive.db");
    let channel = Uuid::new_v4();
    {
        let conn = open_archive_db(&database).unwrap();
        assert!(list(&conn, "owner", "relay").unwrap().is_empty());
        assert_eq!(
            set(&conn, "owner", "relay", channel, true).unwrap(),
            vec![channel]
        );
        // Another identity, or the same identity on another relay, is a
        // different room list entirely.
        assert!(list(&conn, "other", "relay").unwrap().is_empty());
        assert!(list(&conn, "owner", "other").unwrap().is_empty());
    }
    let conn = open_archive_db(&database).unwrap();
    assert_eq!(list(&conn, "owner", "relay").unwrap(), vec![channel]);
}

/// Turning it on twice is the same as turning it on once — the UI toggle can
/// be pressed twice, and a double row would widen nothing but would make the
/// limit count wrong.
#[test]
fn turning_it_on_twice_leaves_one_channel() {
    let directory = tempfile::tempdir().unwrap();
    let conn = open_archive_db(&directory.path().join("archive.db")).unwrap();
    let channel = Uuid::new_v4();
    set(&conn, "owner", "relay", channel, true).unwrap();
    assert_eq!(
        set(&conn, "owner", "relay", channel, true).unwrap(),
        vec![channel]
    );
}

/// Off means off: the channel goes back to mention-only, which is the whole
/// reason the owner can afford to open a room at all.
#[test]
fn turning_it_off_removes_only_that_channel() {
    let directory = tempfile::tempdir().unwrap();
    let conn = open_archive_db(&directory.path().join("archive.db")).unwrap();
    let kept = Uuid::new_v4();
    let dropped = Uuid::new_v4();
    set(&conn, "owner", "relay", kept, true).unwrap();
    set(&conn, "owner", "relay", dropped, true).unwrap();
    let remaining = set(&conn, "owner", "relay", dropped, false).unwrap();
    assert_eq!(remaining, vec![kept]);
}

/// Turning off a channel that was never on is not an error — the caller is
/// asking for a state, not applying a delta.
#[test]
fn turning_off_an_unopened_channel_is_not_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let conn = open_archive_db(&directory.path().join("archive.db")).unwrap();
    assert!(set(&conn, "owner", "relay", Uuid::new_v4(), false)
        .unwrap()
        .is_empty());
}

/// Each open room widens a relay subscription, so the set is bounded and the
/// refusal says what to do instead of failing silently.
#[test]
fn the_limit_refuses_the_next_channel_and_says_why() {
    let directory = tempfile::tempdir().unwrap();
    let conn = open_archive_db(&directory.path().join("archive.db")).unwrap();
    for _ in 0..MAX_CONVERSATION_CHANNELS {
        set(&conn, "owner", "relay", Uuid::new_v4(), true).unwrap();
    }
    let error = set(&conn, "owner", "relay", Uuid::new_v4(), true).unwrap_err();
    assert!(error.contains("turn one off"), "{error}");
    // A channel that is already open is not a new one, so it still succeeds at
    // the limit rather than reporting a failure for a no-op.
    let existing = list(&conn, "owner", "relay").unwrap()[0];
    assert!(set(&conn, "owner", "relay", existing, true).is_ok());
}
