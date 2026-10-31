use super::*;
use crate::provider_limits::LimitWindow;

fn window(used_percent: f64, stale: bool) -> LimitWindow {
    LimitWindow {
        label: "Week".to_string(),
        used_percent,
        resets: Some("Sep 23 at 9am".to_string()),
        resets_at: Some(1_790_157_201),
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

fn on() -> FailoverConfig {
    FailoverConfig {
        enabled: true,
        ..FailoverConfig::default()
    }
}

/// Uprooting a team is the owner's decision; the feature ships off.
#[test]
fn a_disabled_failover_never_routes() {
    let groups = vec![
        group("claude", vec![window(100.0, false)]),
        group("codex", vec![window(5.0, false)]),
    ];
    assert_eq!(plan_route(&FailoverConfig::default(), &groups), None);
    assert!(!FailoverConfig::default().enabled);
}

/// The case this exists for: Claude is walled off, Codex has its own separate
/// limit with room, so the work moves and continues.
#[test]
fn an_exhausted_claude_routes_to_a_healthy_codex() {
    let groups = vec![
        group("claude", vec![window(100.0, false)]),
        group("codex", vec![window(12.0, false)]),
    ];
    assert_eq!(
        plan_route(&on(), &groups),
        Some(Route {
            from: "claude".to_string(),
            to: "codex".to_string()
        })
    );
}

/// It routes both ways: whichever provider is walled off is the one left.
#[test]
fn an_exhausted_codex_routes_to_a_healthy_claude() {
    let groups = vec![
        group("claude", vec![window(20.0, false)]),
        group("codex", vec![window(99.0, false)]),
    ];
    let route = plan_route(&on(), &groups).expect("a route");
    assert_eq!(route.from, "codex");
    assert_eq!(route.to, "claude");
}

/// Moving a team onto a provider that is itself nearly full just relocates the
/// outage, so a marginal destination is refused.
#[test]
fn a_destination_without_real_headroom_is_refused() {
    let groups = vec![
        group("claude", vec![window(100.0, false)]),
        // Under "exhausted" but over "healthy": not worth moving onto.
        group("codex", vec![window(80.0, false)]),
    ];
    assert_eq!(plan_route(&on(), &groups), None);
}

/// A window that already reset still reads full until its runtime reports
/// again. Migrating a whole team over a limit that no longer exists would be
/// worse than the limit.
#[test]
fn a_stale_window_never_triggers_a_move() {
    let groups = vec![
        group("claude", vec![window(100.0, true)]),
        group("codex", vec![window(5.0, false)]),
    ];
    assert_eq!(plan_route(&on(), &groups), None);
}

/// A runtime whose limits cannot be read is never a destination: moving a team
/// onto a provider with unknown headroom is a guess.
#[test]
fn a_runtime_reporting_nothing_is_never_a_destination() {
    let groups = vec![
        group("claude", vec![window(100.0, false)]),
        LimitGroup {
            unavailable: Some("could not run the codex CLI".to_string()),
            ..group("codex", vec![])
        },
    ];
    assert_eq!(plan_route(&on(), &groups), None);
}

/// Nor is a runtime whose only window is stale — that is unknown, not empty.
#[test]
fn a_destination_whose_only_reading_is_stale_is_refused() {
    let groups = vec![
        group("claude", vec![window(100.0, false)]),
        group("codex", vec![window(2.0, true)]),
    ];
    assert_eq!(plan_route(&on(), &groups), None);
}

/// With two healthy destinations the emptiest wins, so a failover buys as much
/// room as the machine has.
#[test]
fn the_emptiest_healthy_destination_wins() {
    let groups = vec![
        group("codex", vec![window(100.0, false)]),
        group("claude", vec![window(10.0, false)]),
    ];
    let route = plan_route(&on(), &groups).expect("a route");
    assert_eq!(route.to, "claude");
}

/// A runtime's fullest window decides it: room in the weekly window is no help
/// when the five-hour one is spent.
#[test]
fn the_fullest_window_decides_a_runtimes_health() {
    let groups = vec![
        group("claude", vec![window(4.0, false), window(97.0, false)]),
        group("codex", vec![window(3.0, false)]),
    ];
    let route = plan_route(&on(), &groups).expect("a route");
    assert_eq!(
        route.from, "claude",
        "a spent five-hour window exhausts the runtime"
    );
}

/// Nothing to escape means nothing to do.
#[test]
fn two_healthy_runtimes_produce_no_route() {
    let groups = vec![
        group("claude", vec![window(10.0, false)]),
        group("codex", vec![window(20.0, false)]),
    ];
    assert_eq!(plan_route(&on(), &groups), None);
}

/// Both walled off means there is nowhere to go, and saying so is better than
/// moving the team into a second wall.
#[test]
fn two_exhausted_runtimes_produce_no_route() {
    let groups = vec![
        group("claude", vec![window(100.0, false)]),
        group("codex", vec![window(99.0, false)]),
    ];
    assert_eq!(plan_route(&on(), &groups), None);
}

/// OpenCode's models are free but materially weaker, so an automatic move
/// there would downgrade the team's output while reporting a success.
#[test]
fn opencode_is_never_an_automatic_destination() {
    let groups = vec![
        group("claude", vec![window(100.0, false)]),
        group("opencode", vec![window(0.0, false)]),
    ];
    assert_eq!(plan_route(&on(), &groups), None);
}

/// Coming home needs real headroom, not merely dipping under "exhausted" —
/// otherwise the team is sent straight back into the wall.
#[test]
fn returning_home_needs_the_same_headroom_as_leaving() {
    let recovered = vec![group("claude", vec![window(8.0, false)])];
    assert!(can_return(&on(), &recovered, "claude"));

    let barely = vec![group("claude", vec![window(80.0, false)])];
    assert!(
        !can_return(&on(), &barely, "claude"),
        "under 'exhausted' is not the same as having room"
    );
}

/// A home runtime that reports nothing cannot be declared ready.
#[test]
fn an_unreadable_home_runtime_is_not_ready() {
    let unknown = vec![LimitGroup {
        unavailable: Some("could not run the claude CLI".to_string()),
        ..group("claude", vec![])
    }];
    assert!(!can_return(&on(), &unknown, "claude"));
    assert!(!can_return(&on(), &[], "claude"), "no reading at all");
}

/// A destination model must be one the workspace actually runs, or the harness
/// silently falls back to its default and the move changes nothing.
#[test]
fn destination_models_are_real_configured_models() {
    assert_eq!(destination_model("codex", true), Some("gpt-5.6-terra"));
    assert_eq!(destination_model("codex", false), Some("gpt-5.6-terra"));
    assert_eq!(destination_model("claude", true), Some("claude-sonnet-5"));
    assert_eq!(
        destination_model("claude", false),
        Some("claude-haiku-4-5-20251001")
    );
    assert_eq!(destination_model("goose", true), None);
}

/// The route and the displaced record cross the wire and the disk; a rename
/// would strand agents on the wrong runtime with no way home.
#[test]
fn the_wire_shapes_match_what_the_ui_and_disk_read() {
    let route = Route {
        from: "claude".to_string(),
        to: "codex".to_string(),
    };
    assert!(serde_json::to_string(&route)
        .unwrap()
        .contains("\"from\":\"claude\""));

    let displaced = Displaced {
        pubkey: "a".to_string(),
        relay_url: "wss://relay.test".to_string(),
        name: "Pilot".to_string(),
        home_runtime: "claude".to_string(),
        home_model: Some("claude-sonnet-5".to_string()),
        ..Displaced::default()
    };
    let json = serde_json::to_string(&displaced).unwrap();
    assert!(json.contains("\"homeRuntime\":\"claude\""), "{json}");
    assert!(json.contains("\"homeModel\":\"claude-sonnet-5\""), "{json}");
    assert_eq!(
        serde_json::from_str::<Displaced>(&json).unwrap(),
        displaced,
        "a record that cannot be read back strands the agent"
    );
}

/// Both store halves must be restorable. A definition has a slug and no relay;
/// an instance has a pubkey and a relay. Losing either half's home would leave
/// the team split across two runtimes with no way back.
#[test]
fn both_store_halves_can_be_recorded_and_read_back() {
    let definition = Displaced {
        slug: "pilot-project-manager".to_string(),
        name: "Pilot".to_string(),
        home_runtime: "claude".to_string(),
        home_model: Some("claude-sonnet-5".to_string()),
        ..Displaced::default()
    };
    let json = serde_json::to_string(&definition).unwrap();
    assert_eq!(
        serde_json::from_str::<Displaced>(&json).unwrap(),
        definition
    );
    assert!(
        definition.pubkey.is_empty() && definition.relay_url.is_empty(),
        "a definition has nothing to restart"
    );

    // An older record written before slugs were stored must still load.
    let legacy = br#"{"pubkey":"a","relayUrl":"wss://r","name":"Pilot","homeRuntime":"claude"}"#;
    let parsed: Displaced = serde_json::from_slice(legacy).unwrap();
    assert_eq!(parsed.slug, "");
    assert_eq!(parsed.home_model, None);
}
