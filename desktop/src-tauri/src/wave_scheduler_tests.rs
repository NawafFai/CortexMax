use super::*;

const NOW: i64 = 1_790_000_000;
const MIN: i64 = 60;

fn agent(name: &str, running: bool, idle_minutes: Option<i64>) -> WaveAgent {
    WaveAgent {
        pubkey: format!("key-{name}"),
        relay_url: "wss://relay.test".to_string(),
        name: name.to_string(),
        running,
        last_turn_at: idle_minutes.map(|m| NOW - m * MIN),
    }
}

fn on(concurrent: usize) -> WaveConfig {
    WaveConfig {
        enabled: true,
        concurrent_agents: concurrent,
        ..WaveConfig::default()
    }
}

/// Only the named coordinating roles hold a place in every wave.
fn leads(name: &str) -> bool {
    matches!(name, "Pilot" | "CORTEX")
}

/// Stopping and starting someone's team is their decision; it ships off.
#[test]
fn a_disabled_scheduler_plans_nothing() {
    let agents = vec![agent("A", true, Some(60)), agent("B", false, Some(60))];
    let plan = plan(&WaveConfig::default(), &agents, NOW, &leads);
    assert!(plan.stop.is_empty() && plan.start.is_empty());
    assert!(!WaveConfig::default().enabled);
}

/// The whole point: a crowd is trimmed to the cap and the rest queue.
#[test]
fn a_team_over_the_cap_is_trimmed_to_it() {
    let agents = vec![
        agent("A", true, Some(60)),
        agent("B", true, Some(60)),
        agent("C", true, Some(60)),
        agent("D", true, Some(60)),
    ];
    let plan = plan(&on(2), &agents, NOW, &leads);
    assert_eq!(plan.stop.len(), 2, "four running, two places");
    assert!(plan.start.is_empty());
}

/// A free place is filled from the queue rather than left empty.
#[test]
fn a_free_place_starts_a_queued_agent() {
    let agents = vec![
        agent("A", true, Some(60)),
        agent("B", false, Some(120)),
        agent("C", false, Some(90)),
    ];
    let plan = plan(&on(3), &agents, NOW, &leads);
    assert!(plan.stop.is_empty());
    assert_eq!(plan.start.len(), 2, "two places free, two queued agents");
}

/// Cutting a turn off wastes everything that turn already spent — the exact
/// cost this module exists to avoid. A busy agent keeps its place even when it
/// is over the cap.
#[test]
fn a_busy_agent_is_never_stopped_to_make_room() {
    let agents = vec![
        agent("A", true, Some(0)),
        agent("B", true, Some(0)),
        agent("C", true, Some(0)),
    ];
    let plan = plan(&on(1), &agents, NOW, &leads);
    assert!(
        plan.stop.is_empty(),
        "all three are mid-turn: {:?}",
        plan.stop
    );
    assert_eq!(plan.held_busy, 2);
}

/// An agent that has never taken a turn may be working on its first one, so
/// it is treated as busy rather than as idle.
#[test]
fn an_agent_with_no_recorded_turn_is_treated_as_busy() {
    let agents = vec![agent("A", true, Some(999)), agent("B", true, None)];
    let plan = plan(&on(1), &agents, NOW, &leads);
    assert!(
        !plan.stop.contains(&"key-B".to_string()),
        "an unknown turn is not an idle one"
    );
}

/// Coordination roles are useless queued behind the work they direct, so they
/// hold a place in every wave.
#[test]
fn a_lead_holds_a_place_over_a_longer_waiting_worker() {
    let agents = vec![
        // Waited far longer, but is not a lead.
        agent("Worker", false, Some(600)),
        agent("Pilot", false, Some(1)),
    ];
    let plan = plan(&on(1), &agents, NOW, &leads);
    assert_eq!(plan.start, vec!["key-Pilot".to_string()]);
}

/// Among non-leads the longest-waiting goes next, so a queue cannot starve one
/// agent forever.
#[test]
fn the_longest_waiting_worker_goes_next() {
    let agents = vec![
        agent("Recent", false, Some(5)),
        agent("Patient", false, Some(300)),
    ];
    let plan = plan(&on(1), &agents, NOW, &leads);
    assert_eq!(plan.start, vec!["key-Patient".to_string()]);
}

/// An agent that has never run has been waiting since the team started, so it
/// sorts ahead of one that ran an hour ago rather than behind it.
#[test]
fn an_agent_that_never_ran_outranks_one_that_ran_an_hour_ago() {
    let agents = vec![agent("Ran", false, Some(60)), agent("Never", false, None)];
    let plan = plan(&on(1), &agents, NOW, &leads);
    assert_eq!(plan.start, vec!["key-Never".to_string()]);
}

/// A team already at the cap is left entirely alone — no churn for its own
/// sake, since every restart costs a fresh session.
#[test]
fn a_team_already_at_the_cap_is_left_alone() {
    let agents = vec![
        agent("A", true, Some(60)),
        agent("B", true, Some(60)),
        agent("C", false, Some(60)),
    ];
    let plan = plan(&on(2), &agents, NOW, &leads);
    assert!(plan.stop.is_empty());
    assert!(plan.start.is_empty(), "no place is free: {:?}", plan.start);
}

/// A cap of zero would stop the whole team, which is Break's job and not
/// something a scheduler should do by arithmetic.
#[test]
fn a_cap_of_zero_plans_nothing_rather_than_stopping_everyone() {
    let agents = vec![agent("A", true, Some(60)), agent("B", true, Some(60))];
    let plan = plan(&on(0), &agents, NOW, &leads);
    assert!(plan.stop.is_empty(), "stopping everyone is Break, not this");
}

/// A cap larger than the team starts everyone and stops no one.
#[test]
fn a_cap_larger_than_the_team_starts_everyone() {
    let agents = vec![agent("A", false, Some(60)), agent("B", false, Some(60))];
    let plan = plan(&on(10), &agents, NOW, &leads);
    assert_eq!(plan.start.len(), 2);
    assert!(plan.stop.is_empty());
}

/// Idleness is the owner's threshold: a stricter one frees places sooner.
#[test]
fn the_idle_threshold_decides_when_a_place_is_free() {
    let agents = vec![agent("A", true, Some(8)), agent("B", true, Some(8))];

    let patient = WaveConfig {
        enabled: true,
        concurrent_agents: 1,
        idle_minutes: 30.0,
    };
    assert!(
        plan(&patient, &agents, NOW, &leads).stop.is_empty(),
        "eight minutes is still working at a 30-minute threshold"
    );

    let brisk = WaveConfig {
        idle_minutes: 5.0,
        ..patient
    };
    assert_eq!(
        plan(&brisk, &agents, NOW, &leads).stop.len(),
        1,
        "at a 5-minute threshold the place is free"
    );
}

/// The plan crosses the wire to the UI; a rename would empty the panel.
#[test]
fn the_wire_shape_matches_what_the_ui_reads() {
    let agents = vec![
        agent("A", true, Some(60)),
        agent("B", true, Some(0)),
        agent("C", true, Some(60)),
    ];
    let json = serde_json::to_string(&plan(&on(1), &agents, NOW, &leads)).unwrap();
    assert!(json.contains("\"heldBusy\":"), "{json}");
    assert!(json.contains("\"stop\":"), "{json}");
    assert!(!json.contains("held_busy"), "{json}");
}

/// The config crosses the wire and the disk.
#[test]
fn the_config_round_trips_in_the_shape_the_ui_reads() {
    let config = WaveConfig {
        enabled: true,
        concurrent_agents: 6,
        idle_minutes: 15.0,
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"concurrentAgents\":6"), "{json}");
    assert!(json.contains("\"idleMinutes\":15.0"), "{json}");
    assert_eq!(serde_json::from_str::<WaveConfig>(&json).unwrap(), config);
}
