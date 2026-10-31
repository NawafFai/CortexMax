use super::*;
use crate::archive::{channel_workspaces, store::open_archive_db};

fn member(seed: u8, role: &str, capabilities: &[&str]) -> ProjectMember {
    ProjectMember {
        pubkey: nostr::Keys::new(nostr::SecretKey::from_slice(&[seed; 32]).unwrap())
            .public_key()
            .to_hex(),
        role: role.into(),
        capabilities: capabilities.iter().map(|value| (*value).into()).collect(),
    }
}

fn team() -> ProjectTeam {
    let coordinator = member(1, "coordinator", &["read", "manage_team"]);
    ProjectTeam {
        coordinator_pubkey: Some(coordinator.pubkey.clone()),
        members: vec![coordinator],
        paused: false,
        ..Default::default()
    }
}

#[test]
fn project_teams_check_seam_rejects_stale_missing_authority_and_other_channels() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let conn = open_archive_db(&root.join("archive.db")).unwrap();
    let channel = Uuid::new_v4();
    channel_workspaces::set(&conn, "owner", "relay", channel, 0, root.to_str()).unwrap();
    let mut draft = team();
    save(&conn, channel, "owner", 0, draft.clone()).unwrap();
    assert!(prepare_check(&conn, "owner", "relay", channel, 1, 1).is_err());
    draft.members[0].capabilities.push("run_checks".into());
    let second = save(&conn, channel, "owner", 1, draft.clone()).unwrap();
    assert_eq!(
        prepare_check(&conn, "owner", "relay", channel, 1, 2)
            .unwrap()
            .0,
        root
    );
    assert!(prepare_check(&conn, "owner", "relay", channel, 1, 1).is_err());
    assert!(prepare_check(&conn, "other", "relay", channel, 1, 2).is_err());
    assert!(prepare_check(&conn, "owner", "other", channel, 1, 2).is_err());
    assert!(prepare_check(&conn, "owner", "relay", Uuid::new_v4(), 1, 2).is_err());
    draft.coordinator_pubkey = None;
    draft.members[0].role = "member".into();
    let unconfigured = save(&conn, channel, "owner", 2, draft).unwrap();
    assert_eq!(unconfigured.members.len(), 1);
    assert!(prepare_check(&conn, "owner", "relay", channel, 1, 3).is_err());
    let mut paused = second;
    paused.paused = true;
    save(&conn, channel, "owner", 3, paused).unwrap();
    assert!(prepare_check(&conn, "owner", "relay", channel, 1, 4).is_err());
    channel_workspaces::set(&conn, "owner", "relay", channel, 1, root.to_str()).unwrap();
    assert!(prepare_check(&conn, "owner", "relay", channel, 1, 4).is_err());
}

fn save(
    conn: &Connection,
    channel: Uuid,
    actor: &str,
    generation: i64,
    next: ProjectTeam,
) -> Result<ProjectTeam, String> {
    set(
        conn,
        TeamUpdate {
            owner: "owner",
            relay: "relay",
            channel,
            actor,
            expected_generation: generation,
            expected_binding_generation: 1,
            available: &HashSet::from([
                member(1, "member", &[]).pubkey,
                member(2, "member", &[]).pubkey,
            ]),
        },
        next,
    )
}

#[test]
fn project_teams_persist_revoke_fence_and_isolate() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let database = root.join("archive.db");
    let channel = Uuid::new_v4();
    let conn = open_archive_db(&database).unwrap();
    channel_workspaces::set(&conn, "owner", "relay", channel, 0, root.to_str()).unwrap();
    let first = save(&conn, channel, "owner", 0, team()).unwrap();
    let coordinator = first.coordinator_pubkey.clone().unwrap();
    authorize(&first, 1, 1, &coordinator, "read").unwrap();
    assert!(authorize(&first, 1, 1, &coordinator, "edit").is_err());
    assert!(save(&conn, channel, "owner", 0, team()).is_err());
    assert!(load(&conn, "other", "relay", channel)
        .unwrap()
        .members
        .is_empty());
    assert!(load(&conn, "owner", "other", channel)
        .unwrap()
        .members
        .is_empty());
    assert!(load(&conn, "owner", "relay", Uuid::new_v4())
        .unwrap()
        .members
        .is_empty());
    drop(conn);
    let conn = open_archive_db(&database).unwrap();
    assert_eq!(
        load(&conn, "owner", "relay", channel).unwrap().members,
        first.members
    );
    let revoked = save(&conn, channel, "owner", 1, ProjectTeam::default()).unwrap();
    assert!(authorize(&revoked, 1, 1, &coordinator, "read").is_err());
    assert!(authorize(&revoked, 1, 2, &coordinator, "read").is_err());
    let restored = save(&conn, channel, "owner", 2, team()).unwrap();
    channel_workspaces::set(&conn, "owner", "relay", channel, 1, root.to_str()).unwrap();
    assert!(authorize(&restored, 2, 3, &coordinator, "read").is_err());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM channel_project_team_revisions",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
}

#[test]
fn project_teams_delegate_only_subset_without_identity_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let conn = open_archive_db(&root.join("archive.db")).unwrap();
    let channel = Uuid::new_v4();
    channel_workspaces::set(&conn, "owner", "relay", channel, 0, root.to_str()).unwrap();
    conn.execute_batch("CREATE TABLE synthetic_global_agents(pubkey TEXT); INSERT INTO synthetic_global_agents VALUES('preserved');").unwrap();
    let first = save(&conn, channel, "owner", 0, team()).unwrap();
    let coordinator = first.coordinator_pubkey.clone().unwrap();
    let mut delegated = first.clone();
    delegated.members.push(member(2, "member", &["read"]));
    assert!(save(&conn, channel, "outsider", 1, delegated.clone()).is_err());
    let mut escalation = delegated.clone();
    escalation.members[1].capabilities.push("edit".into());
    assert!(save(&conn, channel, &coordinator, 1, escalation).is_err());
    let mut self_escalation = delegated.clone();
    self_escalation.members[0].capabilities.push("edit".into());
    assert!(save(&conn, channel, &coordinator, 1, self_escalation).is_err());
    let mut privileged = delegated.clone();
    privileged.members[1].capabilities.push("commit".into());
    assert!(save(&conn, channel, "owner", 1, privileged).is_err());
    let second = save(&conn, channel, &coordinator, 1, delegated).unwrap();
    let mut removed = second;
    removed.members.retain(|grant| grant.pubkey == coordinator);
    save(&conn, channel, &coordinator, 2, removed).unwrap();
    assert_eq!(
        conn.query_row("SELECT pubkey FROM synthetic_global_agents", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "preserved"
    );
}

#[test]
fn project_teams_reject_unknown_agents_and_upgrade_collision() {
    let mut draft = team();
    let available = HashSet::from([draft.members[0].pubkey.clone()]);
    draft.members.push(member(2, "reviewer", &["review"]));
    assert!(validate(&draft, &available).is_err());
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER); CREATE TABLE channel_project_teams(wrong TEXT);").unwrap();
    assert!(migrate(&conn).is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM archive_migrations", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let valid = Connection::open_in_memory().unwrap();
    valid
        .execute_batch(
            "CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER);",
        )
        .unwrap();
    migrate(&valid).unwrap();
    migrate(&valid).unwrap();
}

#[test]
fn project_teams_stale_editor_cannot_grant_access_after_rebind() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let other_root = root.join("other");
    std::fs::create_dir(&other_root).unwrap();
    let conn = open_archive_db(&root.join("archive.db")).unwrap();
    let channel = Uuid::new_v4();
    channel_workspaces::set(&conn, "owner", "relay", channel, 0, root.to_str()).unwrap();
    let original = save(&conn, channel, "owner", 0, team()).unwrap();
    channel_workspaces::set(&conn, "owner", "relay", channel, 1, other_root.to_str()).unwrap();
    let mut stale = original.clone();
    stale.members[0].capabilities.push("edit".into());
    assert!(save(&conn, channel, "owner", 1, stale.clone())
        .unwrap_err()
        .contains("binding changed"));
    let unchanged = load(&conn, "owner", "relay", channel).unwrap();
    assert_eq!(unchanged.generation, original.generation);
    assert_eq!(unchanged.members, original.members);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM channel_project_team_revisions",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let available = HashSet::from([original.members[0].pubkey.clone()]);
    let fresh = set(
        &conn,
        TeamUpdate {
            owner: "owner",
            relay: "relay",
            channel,
            actor: "owner",
            expected_generation: 1,
            expected_binding_generation: 2,
            available: &available,
        },
        stale,
    )
    .unwrap();
    assert_eq!(fresh.binding_generation, 2);
    assert_eq!(fresh.generation, 2);
}
