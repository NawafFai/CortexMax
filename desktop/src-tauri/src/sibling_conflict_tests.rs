use super::*;
use std::collections::BTreeSet;

fn ours(pubkeys: &[&str]) -> BTreeSet<String> {
    pubkeys.iter().map(|k| (*k).to_string()).collect()
}

/// Write a sibling store holding `agents`, plus a private key field that this
/// module must never read.
fn sibling_store(dir: &Path, agents: &[(&str, &str)]) {
    std::fs::create_dir_all(dir).unwrap();
    let rows: Vec<serde_json::Value> = agents
        .iter()
        .map(|(pubkey, name)| {
            serde_json::json!({
                "pubkey": pubkey,
                "name": name,
                "private_key_nsec": "nsec1-this-must-never-be-read",
                "relay_url": "wss://relay.test",
            })
        })
        .collect();
    std::fs::write(
        dir.join("managed-agents.json"),
        serde_json::to_vec(&rows).unwrap(),
    )
    .unwrap();
}

fn write_pid(dir: &Path, pubkey: &str, pid: u32) {
    let pids = dir.join("agent-pids");
    std::fs::create_dir_all(&pids).unwrap();
    std::fs::write(pids.join(format!("{pubkey}.pid")), pid.to_string()).unwrap();
}

/// Sharing identities always matters, because the two apps can never safely
/// run together — but it is only actively breaking things while the sibling
/// is running them.
#[test]
fn severity_separates_a_latent_overlap_from_an_active_one() {
    assert_eq!(assess(0, 0), Severity::None);
    assert_eq!(assess(31, 0), Severity::Warning);
    assert_eq!(assess(31, 1), Severity::Critical);
}

/// The whole point: a sibling holding our keys is found and counted.
#[test]
fn a_sibling_sharing_identities_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join("agents");
    sibling_store(
        &agents,
        &[("a", "Pilot"), ("b", "Judge"), ("z", "Stranger")],
    );

    let report = inspect_sibling("xyz.block.buzz.app", &agents, &ours(&["a", "b"]), &|_| {
        false
    })
    .expect("a conflict");
    assert_eq!(report.shared_agents, 2, "the stranger is not ours");
    assert_eq!(report.running_shared_agents, 0);
    assert_eq!(report.severity, Severity::Warning);
    assert_eq!(report.sample_names, vec!["Pilot", "Judge"]);
}

/// A sibling running one of our agents is the case this feature exists for:
/// two processes signing as one identity on the same relay.
#[test]
fn a_sibling_running_one_of_our_agents_is_critical() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join("agents");
    sibling_store(&agents, &[("a", "Pilot"), ("b", "Judge")]);
    write_pid(&agents, "a", 4242);

    let report = inspect_sibling("sibling", &agents, &ours(&["a", "b"]), &|pid| pid == 4242)
        .expect("a conflict");
    assert_eq!(report.running_shared_agents, 1);
    assert_eq!(report.severity, Severity::Critical);
}

/// A pid file left behind by a crash must not be read as a running agent, or
/// the warning would cry wolf forever after the sibling was used once.
#[test]
fn a_stale_pid_file_is_not_a_running_agent() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join("agents");
    sibling_store(&agents, &[("a", "Pilot")]);
    write_pid(&agents, "a", 4242);

    let report =
        inspect_sibling("sibling", &agents, &ours(&["a"]), &|_| false).expect("a conflict");
    assert_eq!(
        report.running_shared_agents, 0,
        "a dead pid is not a running agent"
    );
    assert_eq!(report.severity, Severity::Warning);
}

/// A running agent the sibling owns alone is not a conflict — only overlap is.
#[test]
fn a_sibling_running_its_own_agents_is_not_a_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join("agents");
    sibling_store(&agents, &[("a", "Ours"), ("z", "Theirs")]);
    write_pid(&agents, "z", 99);

    let report = inspect_sibling("sibling", &agents, &ours(&["a"]), &|_| true).expect("a conflict");
    assert_eq!(report.shared_agents, 1);
    assert_eq!(
        report.running_shared_agents, 0,
        "their own agent running is their business"
    );
}

/// No overlap means no report at all, rather than an empty banner.
#[test]
fn a_sibling_with_no_shared_identities_reports_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join("agents");
    sibling_store(&agents, &[("x", "Someone"), ("y", "Else")]);
    assert!(inspect_sibling("sibling", &agents, &ours(&["a", "b"]), &|_| true).is_none());
}

/// A machine that never installed the sibling must not produce a warning.
#[test]
fn a_missing_sibling_store_reports_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(inspect_sibling(
        "sibling",
        &dir.path().join("does-not-exist"),
        &ours(&["a"]),
        &|_| true
    )
    .is_none());
}

/// A corrupt or half-written sibling store is not a reason to fail; it simply
/// cannot be compared.
#[test]
fn an_unreadable_sibling_store_reports_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(agents.join("managed-agents.json"), b"{ not json").unwrap();
    assert!(inspect_sibling("sibling", &agents, &ours(&["a"]), &|_| true).is_none());
}

/// Key-less rows are definitions, not identities, and cannot conflict.
#[test]
fn definition_rows_are_not_counted_as_identities() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join("agents");
    sibling_store(&agents, &[("", "A definition"), ("a", "An agent")]);

    let report =
        inspect_sibling("sibling", &agents, &ours(&["a"]), &|_| false).expect("a conflict");
    assert_eq!(report.shared_agents, 1);
}

/// The sibling's private key must never enter this process. The store fixture
/// contains one; reading it into any field here would be a credential leak.
#[test]
fn the_siblings_private_key_is_never_deserialized() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join("agents");
    sibling_store(&agents, &[("a", "Pilot")]);

    let report =
        inspect_sibling("sibling", &agents, &ours(&["a"]), &|_| false).expect("a conflict");
    let serialized = serde_json::to_string(&report).unwrap();
    assert!(
        !serialized.contains("nsec"),
        "a private key reached the report: {serialized}"
    );
    // And the narrow struct is what guarantees it: the raw bytes contain the
    // key, but the parsed shape has nowhere to put it.
    let raw = std::fs::read_to_string(agents.join("managed-agents.json")).unwrap();
    assert!(raw.contains("nsec1"), "the fixture must contain a key");
}

/// The report crosses the wire to the UI; a rename would silently hide the
/// banner.
#[test]
fn the_wire_shape_matches_what_the_ui_reads() {
    let report = ConflictReport {
        bundle_id: "xyz.block.buzz.app".to_string(),
        shared_agents: 31,
        running_shared_agents: 2,
        severity: Severity::Critical,
        sample_names: vec!["Pilot".to_string()],
    };
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"sharedAgents\":31"), "{json}");
    assert!(json.contains("\"runningSharedAgents\":2"), "{json}");
    assert!(json.contains("\"severity\":\"critical\""), "{json}");
}
