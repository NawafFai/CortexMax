use super::*;

const HOUR: i64 = 3_600;
const NOW: i64 = 1_790_000_000;

fn candidate(started_ago_hours: f64, idle_minutes: Option<f64>) -> Candidate {
    Candidate {
        pubkey: "npub-test".to_string(),
        relay_url: "wss://relay.test".to_string(),
        name: "Scribe Technical Writer".to_string(),
        started_at: Some(NOW - (started_ago_hours * HOUR as f64) as i64),
        last_turn_at: idle_minutes.map(|minutes| NOW - (minutes * 60.0) as i64),
    }
}

fn on() -> RecyclerConfig {
    RecyclerConfig {
        enabled: true,
        ..RecyclerConfig::default()
    }
}

/// Restarting someone's agents is their decision; the feature ships off.
#[test]
fn a_disabled_recycler_never_recycles() {
    let old_and_quiet = candidate(48.0, Some(600.0));
    assert_eq!(
        verdict(&RecyclerConfig::default(), &old_and_quiet, NOW),
        Verdict::TooYoung
    );
    assert!(!RecyclerConfig::default().enabled);
}

/// The whole point: an old, quiet session is recycled so its context resets.
#[test]
fn an_old_quiet_session_is_recycled() {
    assert_eq!(
        verdict(&on(), &candidate(9.0, Some(45.0)), NOW),
        Verdict::Recycle
    );
}

/// Cutting into a working agent throws away its turn. Age alone must never be
/// enough — this is the failure this module exists to avoid.
#[test]
fn an_old_but_working_session_is_left_alone() {
    assert_eq!(
        verdict(&on(), &candidate(48.0, Some(1.0)), NOW),
        Verdict::Busy,
        "a session still producing turns is doing its job, however old"
    );
}

/// A young session is left alone however quiet, because its context is small
/// and restarting it would buy nothing.
#[test]
fn a_young_session_is_left_alone_however_idle() {
    assert_eq!(
        verdict(&on(), &candidate(1.0, Some(600.0)), NOW),
        Verdict::TooYoung
    );
}

/// "No data" is not "idle". An agent that has never reported a turn might be
/// mid-turn right now, so it is never recycled on a guess.
#[test]
fn an_agent_with_no_recorded_turn_is_never_recycled() {
    assert_eq!(
        verdict(&on(), &candidate(48.0, None), NOW),
        Verdict::Unknown,
        "an unknown last turn must not be read as idle"
    );
}

/// An adopted process has no recorded start, so its session age is unknowable
/// and it must not be restarted on an assumption.
#[test]
fn an_agent_with_no_recorded_start_is_never_recycled() {
    let adopted = Candidate {
        started_at: None,
        ..candidate(48.0, Some(600.0))
    };
    assert_eq!(verdict(&on(), &adopted, NOW), Verdict::Unknown);
}

/// Both boundaries belong to "not yet", so a session is only recycled once it
/// is strictly past both thresholds.
#[test]
fn the_thresholds_are_exclusive_at_their_boundaries() {
    // Exactly at the age threshold, comfortably idle: age passes.
    assert_eq!(
        verdict(&on(), &candidate(8.0, Some(600.0)), NOW),
        Verdict::Recycle
    );
    // A hair under the age threshold.
    assert_eq!(
        verdict(&on(), &candidate(7.99, Some(600.0)), NOW),
        Verdict::TooYoung
    );
    // Old enough, but exactly at the idle threshold: idle passes.
    assert_eq!(
        verdict(&on(), &candidate(9.0, Some(20.0)), NOW),
        Verdict::Recycle
    );
    // A hair under the idle threshold.
    assert_eq!(
        verdict(&on(), &candidate(9.0, Some(19.9)), NOW),
        Verdict::Busy
    );
}

/// The owner's thresholds are theirs: a stricter setting must recycle sooner.
#[test]
fn stricter_thresholds_recycle_sooner() {
    let strict = RecyclerConfig {
        enabled: true,
        max_session_hours: 2.0,
        min_idle_minutes: 5.0,
    };
    let three_hours_quiet = candidate(3.0, Some(10.0));
    assert_eq!(verdict(&strict, &three_hours_quiet, NOW), Verdict::Recycle);
    assert_eq!(
        verdict(&on(), &three_hours_quiet, NOW),
        Verdict::TooYoung,
        "the default leaves the same agent alone"
    );
}

/// A clock that has gone backwards (an NTP correction, a restored snapshot)
/// must not make every session look ancient and trigger a mass restart.
#[test]
fn a_future_start_time_is_not_treated_as_an_old_session() {
    let from_the_future = Candidate {
        started_at: Some(NOW + 10 * HOUR),
        ..candidate(1.0, Some(600.0))
    };
    assert_eq!(verdict(&on(), &from_the_future, NOW), Verdict::TooYoung);
}

/// Defaults match the usage band the machine's own report singles out, so the
/// feature targets the cost it was built for.
#[test]
fn the_defaults_target_the_eight_hour_band() {
    let config = RecyclerConfig::default();
    assert_eq!(config.max_session_hours, 8.0);
    assert!(
        config.min_idle_minutes >= 15.0,
        "too short a quiet window risks cutting into a thinking agent"
    );
}

/// The config crosses the wire to the UI and back to disk.
#[test]
fn the_config_round_trips_in_the_shape_the_ui_reads() {
    let config = RecyclerConfig {
        enabled: true,
        max_session_hours: 6.0,
        min_idle_minutes: 30.0,
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"maxSessionHours\":6.0"), "{json}");
    assert!(json.contains("\"minIdleMinutes\":30.0"), "{json}");
    assert_eq!(
        serde_json::from_str::<RecyclerConfig>(&json).unwrap(),
        config
    );
}
