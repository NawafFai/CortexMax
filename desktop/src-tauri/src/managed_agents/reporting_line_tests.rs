use super::*;

/// The chart the drill used: a general director, two department heads, and a
/// specialist under each. Names are the real ones so a failure reads like the
/// organization it describes.
fn org() -> Vec<ManagedAgentRecord> {
    fn agent(pubkey: &str, name: &str, manager: Option<&str>) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: pubkey.to_string(),
            name: name.to_string(),
            manager_pubkey: manager.map(str::to_string),
            ..Default::default()
        }
    }
    vec![
        agent("p0", "Pilot", None),
        agent("a0", "Axel", Some("p0")),
        agent("o0", "Orion", Some("p0")),
        agent("t0", "Terra", Some("a0")),
        agent("f0", "Forge", Some("a0")),
        agent("n0", "Nova", Some("o0")),
    ]
}

/// The case the feature exists for: a specialist reaching into another
/// department. Nova is not Terra's to direct, so Terra's manager is told.
#[test]
fn another_departments_specialist_is_outside_the_house() {
    let context = reporting_context("t0", &org());
    assert_eq!(context.manager.as_deref(), Some("a0"));
    assert!(
        !context.same_house.contains(&"n0".to_string()),
        "Nova belongs to Orion, not to Terra's house"
    );
}

/// Two engineers in one department working together is the department doing
/// its job. A copy on every such message would bury Axel in his own team's
/// ordinary traffic, and a channel nobody can read protects nobody.
#[test]
fn a_peer_under_the_same_manager_is_inside_the_house() {
    let context = reporting_context("t0", &org());
    assert!(context.same_house.contains(&"f0".to_string()));
}

/// Copying your manager on a message addressed to your manager informs the
/// same person twice.
#[test]
fn your_own_manager_is_inside_the_house() {
    let context = reporting_context("t0", &org());
    assert!(context.same_house.contains(&"a0".to_string()));
}

/// Directing your own people is authority you already hold.
#[test]
fn your_own_reports_are_inside_the_house() {
    let context = reporting_context("a0", &org());
    assert!(context.same_house.contains(&"t0".to_string()));
    assert!(context.same_house.contains(&"f0".to_string()));
}

/// Depth matters: a department head reaching past a supervisor to someone two
/// levels down is still inside their own chain.
#[test]
fn reports_are_found_at_any_depth() {
    let mut roster = org();
    roster.push(ManagedAgentRecord {
        pubkey: "x0".into(),
        name: "Junior".into(),
        manager_pubkey: Some("t0".into()),
        ..Default::default()
    });
    let context = reporting_context("a0", &roster);
    assert!(
        context.same_house.contains(&"x0".to_string()),
        "a report of a report is still this chain's own"
    );
}

/// Two department heads talking to each other are peers, and their shared
/// director is not informed of every conversation between them. But one head
/// reaching into the other's team is exactly the crossing this is about.
#[test]
fn department_heads_are_peers_but_their_teams_are_not_shared() {
    let context = reporting_context("a0", &org());
    assert!(
        context.same_house.contains(&"o0".to_string()),
        "Axel and Orion both report to Pilot"
    );
    assert!(
        !context.same_house.contains(&"n0".to_string()),
        "Orion's specialist is not Axel's to direct"
    );
    assert_eq!(context.manager.as_deref(), Some("p0"));
}

/// The top of the chart reports to the owner, so it has no manager to copy —
/// and must not acquire peers out of everyone else who also has none.
#[test]
fn the_top_of_the_chart_has_no_manager_and_no_peers() {
    let mut roster = org();
    roster.push(ManagedAgentRecord {
        pubkey: "z0".into(),
        name: "Unassigned".into(),
        manager_pubkey: None,
        ..Default::default()
    });
    let context = reporting_context("p0", &roster);
    assert_eq!(context.manager, None);
    assert!(
        !context.same_house.contains(&"z0".to_string()),
        "two agents reporting to the owner are not thereby peers"
    );
    // Its own chain is still its own.
    assert!(context.same_house.contains(&"a0".to_string()));
}

/// Case is not identity. A chart half-written in uppercase would otherwise
/// scatter a department across two houses.
#[test]
fn pubkeys_are_matched_without_regard_to_case() {
    let roster = vec![
        ManagedAgentRecord {
            pubkey: "AA".into(),
            name: "Boss".into(),
            manager_pubkey: None,
            ..Default::default()
        },
        ManagedAgentRecord {
            pubkey: "bb".into(),
            name: "Report".into(),
            manager_pubkey: Some("aa".into()),
            ..Default::default()
        },
    ];
    let context = reporting_context("BB", &roster);
    assert_eq!(context.manager.as_deref(), Some("aa"));
    assert!(context.same_house.contains(&"aa".to_string()));
}

/// A cycle can always reach the stored file by hand. It must cost a wrong
/// answer, never a hang at agent launch.
#[test]
fn a_cycle_in_the_stored_chart_terminates() {
    let roster = vec![
        ManagedAgentRecord {
            pubkey: "a".into(),
            name: "A".into(),
            manager_pubkey: Some("b".into()),
            ..Default::default()
        },
        ManagedAgentRecord {
            pubkey: "b".into(),
            name: "B".into(),
            manager_pubkey: Some("a".into()),
            ..Default::default()
        },
    ];
    let context = reporting_context("a", &roster);
    assert_eq!(context.manager.as_deref(), Some("b"));
    assert!(context.same_house.contains(&"b".to_string()));
}

/// An agent nobody placed reports to the owner and has no house, so nothing
/// it sends is copied anywhere. That is the right default for a chart that
/// has not been drawn yet: silence, not invented structure.
#[test]
fn an_unplaced_agent_has_no_manager_and_an_empty_house() {
    let roster = vec![ManagedAgentRecord {
        pubkey: "solo".into(),
        name: "Solo".into(),
        manager_pubkey: None,
        ..Default::default()
    }];
    assert_eq!(
        reporting_context("solo", &roster),
        ReportingContext::default()
    );
}

/// The value is handed to a spawning process, where an unstable ordering
/// would read as a config change and restart agents for nothing.
#[test]
fn the_house_is_returned_in_a_stable_order() {
    let context = reporting_context("a0", &org());
    let mut sorted = context.same_house.clone();
    sorted.sort();
    assert_eq!(context.same_house, sorted);
}
