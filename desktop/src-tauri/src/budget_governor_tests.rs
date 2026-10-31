use super::*;
use crate::provider_limits::LimitWindow;

fn window(label: &str, used_percent: f64, resets_at: Option<i64>, stale: bool) -> LimitWindow {
    LimitWindow {
        label: label.to_string(),
        used_percent,
        resets: resets_at.map(|_| "Sep 23 at 9am".to_string()),
        resets_at,
        window_minutes: Some(10_080),
        stale,
        model: None,
    }
}

fn group(runtime: &str, windows: Vec<LimitWindow>) -> LimitGroup {
    LimitGroup {
        runtime: runtime.to_string(),
        label: runtime.to_string(),
        windows,
        plan: None,
        note: None,
        unavailable: None,
        age_seconds: None,
    }
}

fn on(action: GovernorAction) -> GovernorConfig {
    GovernorConfig {
        enabled: true,
        threshold_percent: 90.0,
        action,
    }
}

/// Acting on someone's team without being asked is the failure mode worth
/// guarding hardest: the governor does nothing until it is switched on.
#[test]
fn a_disabled_governor_never_fires_however_full_the_window() {
    let groups = vec![group("claude", vec![window("Week", 100.0, Some(1), false)])];
    assert_eq!(decide(&GovernorConfig::default(), &groups, &[]), None);
    assert!(
        !GovernorConfig::default().enabled,
        "the shipped default must be off"
    );
}

/// The default action changes nothing, so switching the governor on cannot by
/// itself stop a team.
#[test]
fn the_default_action_only_notifies() {
    assert_eq!(GovernorConfig::default().action, GovernorAction::Notify);
}

#[test]
fn a_window_below_the_threshold_does_not_fire() {
    let groups = vec![group("claude", vec![window("Week", 89.9, Some(1), false)])];
    assert_eq!(decide(&on(GovernorAction::Break), &groups, &[]), None);
}

#[test]
fn a_window_at_the_threshold_fires() {
    let groups = vec![group("claude", vec![window("Week", 90.0, Some(1), false)])];
    let trigger = decide(&on(GovernorAction::Break), &groups, &[]).expect("a trigger");
    assert_eq!(trigger.runtime, "claude");
    assert!(trigger.reason.contains("90%"), "{}", trigger.reason);
    assert!(
        trigger.reason.contains("resets"),
        "the owner needs to know when it clears: {}",
        trigger.reason
    );
}

/// Codex leaves its last figure behind when it goes idle, so a window that has
/// already reset still reads 99%. Breaking a team over a limit that no longer
/// exists is the worst thing this module could do.
#[test]
fn a_stale_reading_never_fires() {
    let groups = vec![group("codex", vec![window("Week", 99.0, Some(1), true)])];
    assert_eq!(
        decide(&on(GovernorAction::Break), &groups, &[]),
        None,
        "a window that already reset must not stop the team"
    );
}

/// One crossing acts once. Otherwise pressing Start after an automatic Break
/// would be met with an immediate second Break.
#[test]
fn a_window_already_handled_does_not_fire_again() {
    let groups = vec![group("claude", vec![window("Week", 95.0, Some(1), false)])];
    let first = decide(&on(GovernorAction::Break), &groups, &[]).expect("a trigger");
    assert_eq!(
        decide(
            &on(GovernorAction::Break),
            &groups,
            std::slice::from_ref(&first.window_key)
        ),
        None,
        "the owner could never restart their team"
    );
}

/// Next week's window is a different occurrence of the same named limit, and
/// must be allowed to fire — otherwise the governor protects for one week only.
#[test]
fn a_new_window_with_a_later_reset_fires_again() {
    let handled = {
        let groups = vec![group(
            "claude",
            vec![window("Week", 95.0, Some(100), false)],
        )];
        decide(&on(GovernorAction::Break), &groups, &[])
            .expect("a trigger")
            .window_key
    };

    let next_week = vec![group(
        "claude",
        vec![window("Week", 95.0, Some(200), false)],
    )];
    assert!(
        decide(&on(GovernorAction::Break), &next_week, &[handled]).is_some(),
        "a fresh window must be able to fire"
    );
}

/// When several windows cross at once the owner should be told about the worst
/// one, not whichever happened to be listed first.
#[test]
fn the_fullest_crossing_window_is_the_one_reported() {
    let groups = vec![
        group("claude", vec![window("Session", 91.0, Some(1), false)]),
        group("codex", vec![window("Week", 99.0, Some(2), false)]),
    ];
    let trigger = decide(&on(GovernorAction::Economy), &groups, &[]).expect("a trigger");
    assert_eq!(trigger.runtime, "codex");
    assert_eq!(trigger.used_percent, 99.0);
}

/// A runtime that could not be read has no windows and must not be treated as
/// a crossing — "unknown" is not "full".
#[test]
fn an_unavailable_runtime_cannot_fire() {
    let groups = vec![LimitGroup {
        runtime: "claude".to_string(),
        label: "Claude".to_string(),
        windows: Vec::new(),
        plan: None,
        note: None,
        unavailable: Some("could not run the claude CLI".to_string()),
        age_seconds: None,
    }];
    assert_eq!(decide(&on(GovernorAction::Break), &groups, &[]), None);
}

/// The key identifies a window occurrence. Two runtimes at the same percent
/// with the same window name are still two separate crossings.
#[test]
fn keys_distinguish_runtimes_sharing_a_window_name() {
    let groups = vec![
        group("claude", vec![window("Week", 95.0, Some(1), false)]),
        group("codex", vec![window("Week", 95.0, Some(1), false)]),
    ];
    let first = decide(&on(GovernorAction::Break), &groups, &[]).expect("a trigger");
    let second = decide(
        &on(GovernorAction::Break),
        &groups,
        std::slice::from_ref(&first.window_key),
    )
    .expect("the other runtime is a separate crossing");
    assert_ne!(first.window_key, second.window_key);
    assert_ne!(first.runtime, second.runtime);
}

/// A window with no reset time at all still keys stably, so it fires once
/// rather than on every poll.
#[test]
fn a_window_without_a_reset_time_still_fires_only_once() {
    let groups = vec![group(
        "claude",
        vec![LimitWindow {
            label: "Session".to_string(),
            used_percent: 99.0,
            resets: None,
            resets_at: None,
            window_minutes: None,
            stale: false,
            model: None,
        }],
    )];
    let trigger = decide(&on(GovernorAction::Break), &groups, &[]).expect("a trigger");
    assert_eq!(
        decide(&on(GovernorAction::Break), &groups, &[trigger.window_key]),
        None
    );
}

/// The threshold is the owner's, so a stricter one must fire earlier.
#[test]
fn a_lower_threshold_fires_earlier() {
    let groups = vec![group("claude", vec![window("Week", 55.0, Some(1), false)])];
    let strict = GovernorConfig {
        enabled: true,
        threshold_percent: 50.0,
        action: GovernorAction::Notify,
    };
    assert!(decide(&strict, &groups, &[]).is_some());
    assert_eq!(decide(&on(GovernorAction::Notify), &groups, &[]), None);
}

/// Config crosses the wire to the UI and back to disk; a rename would silently
/// reset an owner's settings to the defaults.
#[test]
fn the_config_round_trips_in_the_shape_the_ui_reads() {
    let config = GovernorConfig {
        enabled: true,
        threshold_percent: 85.0,
        action: GovernorAction::Break,
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"thresholdPercent\":85.0"), "{json}");
    assert!(json.contains("\"action\":\"break\""), "{json}");
    assert_eq!(
        serde_json::from_str::<GovernorConfig>(&json).unwrap(),
        config
    );
}
