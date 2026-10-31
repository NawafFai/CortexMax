use super::*;

fn window(model: Option<&str>, used: f64, resets_at: Option<i64>, stale: bool) -> LimitWindow {
    LimitWindow {
        label: model.unwrap_or("all").to_string(),
        used_percent: used,
        resets: None,
        resets_at,
        window_minutes: Some(10_080),
        stale,
        model: model.map(str::to_string),
    }
}

fn group(runtime: &str, windows: Vec<LimitWindow>, note: Option<&str>) -> LimitGroup {
    LimitGroup {
        runtime: runtime.to_string(),
        label: runtime.to_string(),
        windows,
        plan: None,
        note: note.map(str::to_string),
        unavailable: None,
        age_seconds: None,
    }
}

fn member(name: &str, runtime: &str, model: &str) -> Member {
    Member {
        pubkey: format!("pk-{name}"),
        name: name.to_string(),
        runtime: runtime.to_string(),
        model: model.to_string(),
        designated_cover: None,
    }
}

const NOW: i64 = 1_800_000_000;

/// The case this exists for: an agent whose model is spent is named as away,
/// with the provider's own reset as the return date.
#[test]
fn an_agent_on_a_spent_model_goes_on_leave_until_its_reset() {
    let groups = vec![group(
        "claude",
        vec![
            window(Some("Opus"), 100.0, Some(NOW + 7_200), false),
            window(None, 40.0, None, false),
        ],
        None,
    )];
    let members = vec![
        member("Sara", "claude", "claude-opus-5"),
        member("Omar", "kimi", "kimi-code/k3"),
    ];

    let calls = roll_call(&groups, &members);
    assert_eq!(calls.len(), 1, "only the agent on the spent model is away");
    assert_eq!(calls[0].name, "Sara");
    assert_eq!(calls[0].cause, Cause::ModelSpent("Opus".to_string()));
    assert_eq!(calls[0].returns_at, Some(NOW + 7_200));
    assert!(describe(&calls[0], NOW).contains("back in 2h"));
}

/// A provider with no credit has nothing to count down to. Showing a
/// countdown there would promise a return that no clock will bring.
#[test]
fn no_credit_is_leave_without_a_return_date() {
    let groups = vec![group(
        "codex",
        vec![window(None, 99.0, Some(NOW + 3_600), false)],
        Some("Codex reported no credit left"),
    )];
    let members = vec![member("Kernel", "codex", "gpt-5.6-terra")];

    let calls = roll_call(&groups, &members);
    assert_eq!(calls[0].cause, Cause::NoCredit);
    assert_eq!(
        calls[0].returns_at, None,
        "a balance does not refill on a clock"
    );
    assert!(describe(&calls[0], NOW).contains("no reset scheduled"));
}

/// A stale window describes a limit that already reset. Marking an agent away
/// on it would keep a working agent benched for days.
#[test]
fn a_stale_window_does_not_put_anyone_on_leave() {
    let groups = vec![group(
        "claude",
        vec![window(Some("Opus"), 100.0, Some(NOW - 60), true)],
        None,
    )];
    let members = vec![member("Sara", "claude", "claude-opus-5")];
    assert!(roll_call(&groups, &members).is_empty());
}

/// The narrower fact wins: a spent model with account headroom is reported as
/// the model's problem, because that is the one the owner can act on.
#[test]
fn a_spent_model_is_reported_before_the_provider() {
    let groups = vec![group(
        "claude",
        vec![
            window(Some("Opus"), 100.0, Some(NOW + 600), false),
            window(None, 50.0, Some(NOW + 9_000), false),
        ],
        None,
    )];
    let members = vec![member("Sara", "claude", "claude-opus-5")];
    let calls = roll_call(&groups, &members);
    assert_eq!(calls[0].cause, Cause::ModelSpent("Opus".to_string()));
    assert_eq!(
        calls[0].returns_at,
        Some(NOW + 600),
        "the model's reset, not the account's"
    );
}

/// A provider-wide window takes everyone on it, whatever model they hold.
#[test]
fn a_spent_provider_takes_every_agent_on_it() {
    let groups = vec![group(
        "claude",
        vec![window(None, 97.0, Some(NOW + 60), false)],
        None,
    )];
    let members = vec![
        member("Sara", "claude", "claude-opus-5"),
        member("Lina", "claude", "claude-haiku-4-5-20251001"),
    ];
    let calls = roll_call(&groups, &members);
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|c| c.cause == Cause::ProviderSpent));
}

/// A cover on the same spent provider would be away by morning, so a peer
/// elsewhere is preferred and the reason is carried with the suggestion.
#[test]
fn a_cover_is_preferred_from_a_provider_that_still_has_room() {
    let groups = vec![group(
        "claude",
        vec![window(None, 99.0, Some(NOW + 60), false)],
        None,
    )];
    let members = vec![
        member("Sara", "claude", "claude-opus-5"),
        member("Omar", "kimi", "kimi-code/k3"),
    ];
    match &roll_call(&groups, &members)[0].cover {
        Cover::Suggested { name, because } => {
            assert_eq!(name, "Omar");
            assert!(
                because.contains("kimi"),
                "the reason names the provider: {because}"
            );
        }
        other => panic!("expected a suggestion, got {other:?}"),
    }
}

/// The owner's choice outranks the suggestion, even when a different provider
/// would have been picked.
#[test]
fn a_designated_cover_wins_over_the_suggestion() {
    let groups = vec![group("claude", vec![window(None, 99.0, None, false)], None)];
    let mut sara = member("Sara", "claude", "claude-opus-5");
    sara.designated_cover = Some("Scribe".to_string());
    let members = vec![sara, member("Omar", "kimi", "kimi-code/k3")];
    assert_eq!(
        roll_call(&groups, &members)[0].cover,
        Cover::Designated("Scribe".to_string())
    );
}

/// With everyone away, saying so beats naming someone who cannot work.
#[test]
fn no_cover_is_stated_plainly_when_everyone_is_away() {
    let groups = vec![group("claude", vec![window(None, 99.0, None, false)], None)];
    let members = vec![
        member("Sara", "claude", "claude-opus-5"),
        member("Lina", "claude", "claude-sonnet-5"),
    ];
    let calls = roll_call(&groups, &members);
    assert_eq!(calls.len(), 2);
    for call in &calls {
        match &call.cover {
            Cover::None { because } => assert!(!because.is_empty()),
            other => panic!("nobody can cover, got {other:?}"),
        }
    }
}

/// A reset already past means due back, not away for a negative time.
#[test]
fn a_reset_in_the_past_reads_as_due_back() {
    let leave = Leave {
        pubkey: "pk".to_string(),
        name: "Sara".to_string(),
        cause: Cause::ProviderSpent,
        returns_at: Some(NOW - 30),
        cover: Cover::None {
            because: "x".to_string(),
        },
    };
    assert!(describe(&leave, NOW).contains("due back now"));
}

/// An agent on a provider the panel knows nothing about is not on leave:
/// silence about a limit is not evidence of one.
#[test]
fn an_unknown_provider_is_not_treated_as_spent() {
    let groups = vec![group("claude", vec![window(None, 99.0, None, false)], None)];
    let members = vec![member("Solo", "opencode", "opencode/nemotron")];
    assert!(roll_call(&groups, &members).is_empty());
}

#[test]
fn durations_read_the_way_a_person_waits() {
    assert_eq!(humanise(30), "under a minute");
    assert_eq!(humanise(600), "10m");
    assert_eq!(humanise(3_600), "1h");
    assert_eq!(humanise(5_400), "1h 30m");
    assert_eq!(humanise(86_400), "1d");
    assert_eq!(humanise(180_000), "2d 2h");
}
