use super::*;

fn window(label: &str, model: Option<&str>, used_percent: f64, stale: bool) -> LimitWindow {
    LimitWindow {
        label: label.to_string(),
        used_percent,
        resets: None,
        resets_at: None,
        // Weekly, so `account_window` recognises an account-wide one.
        window_minutes: Some(10_080),
        stale,
        model: model.map(str::to_string),
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

fn agent(name: &str, runtime: &str, model: &str) -> Candidate {
    Candidate {
        pubkey: format!("pk-{name}"),
        relay_url: "wss://relay.test".to_string(),
        name: name.to_string(),
        runtime: runtime.to_string(),
        model: model.to_string(),
    }
}

/// The case the feature exists for: one model spent, the subscription not.
#[test]
fn a_spent_model_moves_its_agents_to_a_healthy_sibling() {
    let groups = vec![group(
        "claude",
        vec![
            window("Week · Opus", Some("claude-opus-5"), 100.0, false),
            window("Week · all models", None, 72.0, false),
        ],
    )];
    let roster = vec![
        agent("Sara", "claude", "claude-opus-5"),
        agent("Omar", "claude", "claude-opus-5"),
        agent("Lina", "claude", "claude-sonnet-5"),
    ];
    match plan(&groups, &roster) {
        Plan::Move(moves) => {
            assert_eq!(moves.len(), 2, "only agents on the spent model move");
            // The destination is the first unspent rung of the ladder, which
            // is the closest capability still available — not the cheapest.
            assert!(moves.iter().all(|m| m.to_model == "claude-fable-5-1"));
            assert!(moves.iter().all(|m| m.from_model == "claude-opus-5"));
            assert!(
                moves.iter().all(|m| m.to_model != m.from_model),
                "a move must leave the spent model"
            );
            assert!(
                !moves.iter().any(|m| m.name == "Lina"),
                "an agent already on a healthy model is left alone"
            );
        }
        other => panic!("expected a move, got {other:?}"),
    }
}

/// The whole argument for this module over the cross-provider one: when the
/// account is spent too, a within-provider move lands on the same wall.
#[test]
fn a_spent_account_blocks_the_move_rather_than_shuffling_models() {
    let groups = vec![group(
        "claude",
        vec![
            window("Week · Opus", Some("claude-opus-5"), 100.0, false),
            window("Week · all models", None, 97.0, false),
        ],
    )];
    let roster = vec![agent("Sara", "claude", "claude-opus-5")];
    assert_eq!(
        plan(&groups, &roster),
        Plan::Blocked(Blocked::AccountAlsoSpent)
    );
}

/// A window past its reset describes a limit that no longer exists. Acting on
/// it would migrate a team away from nothing.
#[test]
fn a_stale_spent_window_is_not_a_reason_to_move() {
    let groups = vec![group(
        "claude",
        vec![
            window("Week · Opus", Some("claude-opus-5"), 100.0, true),
            window("Week · all models", None, 40.0, false),
        ],
    )];
    let roster = vec![agent("Sara", "claude", "claude-opus-5")];
    assert_eq!(plan(&groups, &roster), Plan::Blocked(Blocked::NothingSpent));
}

/// No account-wide window means no evidence of headroom, and moving without
/// evidence is guessing.
#[test]
fn a_spent_model_with_no_account_window_does_not_move() {
    let groups = vec![group(
        "claude",
        vec![window("Week · Opus", Some("claude-opus-5"), 100.0, false)],
    )];
    let roster = vec![agent("Sara", "claude", "claude-opus-5")];
    assert_eq!(
        plan(&groups, &roster),
        Plan::Blocked(Blocked::AccountAlsoSpent)
    );
}

#[test]
fn every_sibling_being_spent_blocks_the_move() {
    let groups = vec![group(
        "claude",
        vec![
            window("Week · Opus", Some("claude-opus-5"), 100.0, false),
            window("Week · Sonnet", Some("claude-sonnet-5"), 99.0, false),
            window(
                "Week · Haiku",
                Some("claude-haiku-4-5-20251001"),
                96.0,
                false,
            ),
            window("Week · Fable", Some("claude-fable-5-1"), 98.0, false),
            window("Week · all models", None, 60.0, false),
        ],
    )];
    let roster = vec![agent("Sara", "claude", "claude-opus-5")];
    assert_eq!(
        plan(&groups, &roster),
        Plan::Blocked(Blocked::NoHealthySibling)
    );
}

#[test]
fn a_spent_model_nobody_is_using_is_not_a_move() {
    let groups = vec![group(
        "claude",
        vec![
            window("Week · Opus", Some("claude-opus-5"), 100.0, false),
            window("Week · all models", None, 50.0, false),
        ],
    )];
    let roster = vec![agent("Sara", "claude", "claude-sonnet-5")];
    assert_eq!(
        plan(&groups, &roster),
        Plan::Blocked(Blocked::NoAgentsOnSpentModel)
    );
}

/// Just under the threshold is not spent — the bar has to mean something.
#[test]
fn a_model_below_the_threshold_is_not_spent() {
    let groups = vec![group(
        "claude",
        vec![
            window("Week · Opus", Some("claude-opus-5"), SPENT_AT - 0.1, false),
            window("Week · all models", None, 20.0, false),
        ],
    )];
    let roster = vec![agent("Sara", "claude", "claude-opus-5")];
    assert_eq!(plan(&groups, &roster), Plan::Blocked(Blocked::NothingSpent));
}

#[test]
fn exactly_the_threshold_counts_as_spent() {
    let groups = vec![group(
        "claude",
        vec![
            window("Week · Opus", Some("claude-opus-5"), SPENT_AT, false),
            window("Week · all models", None, 20.0, false),
        ],
    )];
    let roster = vec![agent("Sara", "claude", "claude-opus-5")];
    assert!(matches!(plan(&groups, &roster), Plan::Move(_)));
}

/// "all models" is the account-wide window, not a model that can be left.
#[test]
fn the_account_wide_window_is_never_treated_as_a_model() {
    let groups = vec![group(
        "claude",
        vec![window("Week · all models", None, 100.0, false)],
    )];
    let roster = vec![agent("Sara", "claude", "claude-opus-5")];
    assert_eq!(plan(&groups, &roster), Plan::Blocked(Blocked::NothingSpent));
}

/// A runtime with no known sibling list is left alone rather than guessed at.
#[test]
fn a_runtime_with_no_known_siblings_is_never_moved() {
    assert!(siblings("codex").is_empty());
    let groups = vec![group(
        "codex",
        vec![
            window("Week · something", Some("gpt-5.6-terra"), 100.0, false),
            window("Week · all", None, 10.0, false),
        ],
    )];
    let roster = vec![agent("Sara", "codex", "gpt-5.6-terra")];
    assert_eq!(
        plan(&groups, &roster),
        Plan::Blocked(Blocked::NoHealthySibling)
    );
}

#[test]
fn an_empty_roster_plans_nothing() {
    let groups = vec![group(
        "claude",
        vec![
            window("Week · Opus", Some("claude-opus-5"), 100.0, false),
            window("Week · all models", None, 10.0, false),
        ],
    )];
    assert_eq!(
        plan(&groups, &[]),
        Plan::Blocked(Blocked::NoAgentsOnSpentModel)
    );
}

#[test]
fn no_limits_at_all_plans_nothing() {
    assert_eq!(plan(&[], &[]), Plan::Blocked(Blocked::NothingSpent));
}

/// Each blocked reason has to read as its own sentence, because the point of
/// distinguishing them is to stop the owner looking in the wrong place.
#[test]
fn every_outcome_has_its_own_explanation() {
    let messages = [
        summarize(&Plan::Blocked(Blocked::NothingSpent)),
        summarize(&Plan::Blocked(Blocked::AccountAlsoSpent)),
        summarize(&Plan::Blocked(Blocked::NoHealthySibling)),
        summarize(&Plan::Blocked(Blocked::NoAgentsOnSpentModel)),
    ];
    for message in &messages {
        assert!(!message.is_empty());
    }
    let unique: std::collections::BTreeSet<&String> = messages.iter().collect();
    assert_eq!(
        unique.len(),
        messages.len(),
        "reasons must not share wording"
    );
}

#[test]
fn a_move_is_summarised_with_both_models_named() {
    let plan = Plan::Move(vec![Move {
        pubkey: "pk".to_string(),
        relay_url: "wss://relay.test".to_string(),
        name: "Sara".to_string(),
        runtime: "claude".to_string(),
        from_model: "claude-opus-5".to_string(),
        to_model: "claude-sonnet-5".to_string(),
    }]);
    let message = summarize(&plan);
    assert!(message.contains("claude-opus-5"));
    assert!(message.contains("claude-sonnet-5"));
}

/// The defect this guards against shipped once: the provider writes a display
/// name and the agent record holds an id, and comparing them for equality
/// meant the failover could never fire in the case it was built for.
#[test]
fn a_display_name_matches_the_model_id_it_names() {
    assert!(model_matches("Fable", "claude-fable-5-1"));
    assert!(model_matches("Opus", "claude-opus-5"));
    assert!(model_matches("sonnet", "claude-sonnet-5"));
    assert!(!model_matches("Opus", "claude-sonnet-5"));
    assert!(!model_matches("", "claude-opus-5"));
}

/// The live panel that exposed the bug: `Week · Fable` spent at 100% while
/// `Week · all models` still had room, with agents actually on the id.
#[test]
fn a_spent_display_name_moves_agents_holding_the_matching_id() {
    let groups = vec![group(
        "claude",
        vec![
            window("Week · Fable", Some("Fable"), 100.0, false),
            window("Week · all models", None, 75.0, false),
        ],
    )];
    let roster = vec![
        agent("Sara", "claude", "claude-fable-5-1"),
        agent("Omar", "claude", "claude-haiku-4-5-20251001"),
    ];
    match plan(&groups, &roster) {
        Plan::Move(moves) => {
            assert_eq!(moves.len(), 1, "only the agent on Fable moves");
            assert_eq!(moves[0].name, "Sara");
            assert_ne!(
                moves[0].to_model, "claude-fable-5-1",
                "the destination must not be the spent model"
            );
        }
        other => panic!("expected a move, got {other:?}"),
    }
}

/// A Claude model that is not on the switch ladder still belongs to Claude.
/// Treating it as runtime-less would hide it from the roster entirely.
#[test]
fn an_unlisted_claude_model_is_still_recognised_as_claude() {
    assert_eq!(runtime_of("claude-some-future-6"), "claude");
    assert_eq!(runtime_of("gpt-5.6-terra"), "");
    assert_eq!(runtime_of("kimi-code/k3"), "");
}

/// The fallback list names models the Claude adapter actually offers. Since
/// adapter 0.81 its Opus option is 5.5 and Opus 5 is gone, so a move "down to
/// Opus 5" would ask for a model that is no longer there.
#[test]
fn the_claude_fallback_names_the_opus_the_adapter_offers() {
    let chain = siblings("claude");
    assert!(chain.contains(&"claude-opus-5-5"), "{chain:?}");
    assert!(!chain.contains(&"claude-opus-5"), "{chain:?}");
}
