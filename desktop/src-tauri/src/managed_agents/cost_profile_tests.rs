use super::*;

/// A record carrying only the fields a profile reads or writes. Built through
/// serde so every other field takes the same default a real store load gives
/// it, and so a new required field fails here rather than silently defaulting.
///
/// `keyed` picks the half of the store: an instance has a pubkey, a definition
/// does not.
fn record(
    name: &str,
    runtime: Option<&str>,
    model: Option<&str>,
    keyed: bool,
) -> ManagedAgentRecord {
    serde_json::from_value(serde_json::json!({
        "pubkey": if keyed { "npub-test" } else { "" },
        "name": name,
        "relay_url": "wss://relay.test",
        "acp_command": "",
        "agent_command": "",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 0,
        "system_prompt": null,
        "created_at": "2026-09-19T00:00:00Z",
        "updated_at": "2026-09-19T00:00:00Z",
        "runtime": runtime,
        "model": model,
    }))
    .expect("test record should deserialize")
}

/// A coordinating role keeps the strong model precisely because the rest of
/// the team's work is shaped by its output.
#[test]
fn balanced_gives_deep_roles_sonnet_and_everyone_else_haiku() {
    let profile = CostProfile::Balanced;
    assert_eq!(
        profile.model_for("claude", true),
        Some(CLAUDE_SONNET),
        "a manager on the cheap tier costs more than it saves"
    );
    assert_eq!(profile.model_for("claude", false), Some(CLAUDE_HAIKU));
}

/// Economy is the button pressed while a run is burning a limit: it must not
/// leave an expensive tier anywhere.
#[test]
fn economy_puts_every_claude_role_on_the_cheap_tier() {
    assert_eq!(
        CostProfile::Economy.model_for("claude", true),
        Some(CLAUDE_HAIKU)
    );
    assert_eq!(
        CostProfile::Economy.model_for("claude", false),
        Some(CLAUDE_HAIKU)
    );
    assert_eq!(CostProfile::Economy.parallelism(true), 1);
    assert_eq!(CostProfile::Economy.parallelism(false), 1);
}

/// Every profile must be strictly cheaper than the one above it, or the
/// control does not do what its name promises.
#[test]
fn concurrency_never_decreases_from_economy_to_max() {
    for deep in [true, false] {
        let economy = CostProfile::Economy.parallelism(deep);
        let balanced = CostProfile::Balanced.parallelism(deep);
        let max = CostProfile::Max.parallelism(deep);
        assert!(
            economy <= balanced && balanced <= max,
            "profiles out of order for deep={deep}: {economy}/{balanced}/{max}"
        );
    }
}

/// Only models this workspace has actually run may appear in a tier. An ID the
/// harness cannot match falls back to the agent default with a warning, so an
/// invented "premium" ID would look applied while changing nothing.
#[test]
fn every_tier_uses_a_model_the_harness_can_match() {
    let known = [
        CLAUDE_OPUS,
        CLAUDE_SONNET,
        CLAUDE_HAIKU,
        CODEX_MODEL,
        OPENCODE_MODEL,
    ];
    for profile in [
        CostProfile::Economy,
        CostProfile::Balanced,
        CostProfile::Max,
    ] {
        for runtime in ["claude", "codex", "opencode"] {
            for deep in [true, false] {
                let model = profile
                    .model_for(runtime, deep)
                    .expect("a known runtime always resolves a model");
                assert!(known.contains(&model), "unknown model id {model}");
            }
        }
    }
}

/// Codex and OpenCode have one configured model each, so a profile may change
/// their concurrency but must never claim a tier move.
#[test]
fn single_model_runtimes_keep_their_model_across_every_profile() {
    for profile in [
        CostProfile::Economy,
        CostProfile::Balanced,
        CostProfile::Max,
    ] {
        assert_eq!(profile.model_for("codex", true), Some(CODEX_MODEL));
        assert_eq!(profile.model_for("codex", false), Some(CODEX_MODEL));
        assert_eq!(profile.model_for("opencode", true), Some(OPENCODE_MODEL));
    }
    assert!(CostProfile::is_single_model_runtime("codex"));
    assert!(!CostProfile::is_single_model_runtime("claude"));
}

/// An unrecognized runtime is left exactly as it was, and counted, rather than
/// being rewritten with a model that runtime cannot run.
#[test]
fn an_unknown_runtime_is_skipped_not_rewritten() {
    let mut row = record("Someone", Some("goose"), Some("goose-model"), true);
    assert_eq!(apply_to(&mut row, CostProfile::Economy), Applied::Skipped);
    assert_eq!(row.model.as_deref(), Some("goose-model"), "left untouched");
}

/// A definition carries `definition_parallelism`; the instance carries the
/// `parallelism` spawn actually reads. Writing the wrong one leaves the
/// profile half-applied.
#[test]
fn a_definition_and_an_instance_each_get_their_own_concurrency_field() {
    let mut definition = record("Scribe Technical Writer", Some("claude"), None, false);
    apply_to(&mut definition, CostProfile::Max);
    assert_eq!(definition.definition_parallelism, Some(2));
    assert_eq!(
        definition.parallelism,
        record("untouched", None, None, false).parallelism,
        "an instance field must not be written on a definition"
    );

    let mut instance = record("Scribe Technical Writer", Some("claude"), None, true);
    apply_to(&mut instance, CostProfile::Max);
    assert_eq!(instance.parallelism, 2);
    assert_eq!(instance.definition_parallelism, None);
}

/// Re-pressing the active profile must report "nothing changed" rather than
/// rewriting timestamps and implying work happened.
#[test]
fn applying_the_same_profile_twice_reports_no_second_change() {
    let mut row = record("Pilot Project Manager", Some("claude"), None, false);
    assert_eq!(apply_to(&mut row, CostProfile::Balanced), Applied::Retiered);
    assert_eq!(row.model.as_deref(), Some(CLAUDE_SONNET));
    assert_eq!(
        apply_to(&mut row, CostProfile::Balanced),
        Applied::Unchanged,
        "re-pressing the active profile must not imply work happened"
    );
}

/// A single-model runtime whose concurrency moved is reported as capped, not
/// retiered — the outcome is what the UI tells the owner it did.
#[test]
fn a_codex_change_is_reported_as_capped_not_retiered() {
    let mut row = record(
        "Aegis Cybersecurity Specialist",
        Some("codex"),
        Some(CODEX_MODEL),
        true,
    );
    row.parallelism = 8;
    assert_eq!(apply_to(&mut row, CostProfile::Economy), Applied::Capped);
    assert_eq!(row.parallelism, 1);
}

/// Definitions and instances come in pairs; naming both would read as twice
/// the team in the confirmation message.
#[test]
fn examples_name_each_agent_once() {
    let mut tally = BTreeMap::new();
    for keyed in [false, true] {
        let mut row = record("Prism Software Architect", Some("claude"), None, keyed);
        let applied = apply_to(&mut row, CostProfile::Economy);
        note(&mut tally, &row.name, applied);
    }
    let outcome = summarize(tally);
    assert_eq!(
        outcome.retiered, 1,
        "one agent, even though both of its records were written"
    );
    assert_eq!(
        outcome.examples,
        vec!["Prism Software Architect".to_string()]
    );
}

/// The active-profile probe must not answer from runtimes that match every
/// profile, or an all-Codex team would report whichever profile is checked
/// first.
#[test]
fn a_team_with_only_single_model_runtimes_matches_no_profile() {
    let rows = vec![record("Aegis", Some("codex"), Some(CODEX_MODEL), false)];
    for profile in [
        CostProfile::Economy,
        CostProfile::Balanced,
        CostProfile::Max,
    ] {
        assert!(
            !matches_profile(&rows, profile),
            "codex alone cannot identify a profile"
        );
    }
}

/// The probe identifies the profile a store already sits on, so the UI can
/// highlight it instead of guessing.
#[test]
fn the_probe_recognizes_the_profile_the_store_is_on() {
    let mut rows = vec![
        record("Pilot Project Manager", Some("claude"), None, false),
        record("Scribe Technical Writer", Some("claude"), None, false),
    ];
    for row in &mut rows {
        apply_to(row, CostProfile::Balanced);
    }

    assert!(matches_profile(&rows, CostProfile::Balanced));
    assert!(!matches_profile(&rows, CostProfile::Economy));
    assert!(!matches_profile(&rows, CostProfile::Max));
}

/// A half-applied store belongs to no profile — reporting one would tell the
/// owner a policy is in force when it is not.
#[test]
fn a_mixed_store_matches_no_profile() {
    let mut deep = record("Pilot Project Manager", Some("claude"), None, false);
    apply_to(&mut deep, CostProfile::Balanced);
    let fast = record(
        "Scribe Technical Writer",
        Some("claude"),
        Some(CLAUDE_OPUS),
        false,
    );

    let rows = vec![deep, fast];
    for profile in [
        CostProfile::Economy,
        CostProfile::Balanced,
        CostProfile::Max,
    ] {
        assert!(!matches_profile(&rows, profile));
    }
}

/// The wire shape is what the UI reads; a rename here silently breaks the
/// confirmation message.
#[test]
fn the_outcome_serializes_in_the_shape_the_ui_reads() {
    let outcome = CostProfileOutcome {
        retiered: 3,
        capped_only: 2,
        unchanged: 1,
        skipped: 0,
        examples: vec!["Pilot Project Manager".to_string()],
    };
    let json = serde_json::to_string(&outcome).unwrap();
    assert!(json.contains("\"cappedOnly\":2"), "{json}");
    assert!(!json.contains("capped_only"), "{json}");
}

/// The profile itself crosses the wire; kebab-case is what the UI sends back.
#[test]
fn profiles_round_trip_in_kebab_case() {
    let json = serde_json::to_string(&CostProfile::Economy).unwrap();
    assert_eq!(json, "\"economy\"");
    let parsed: CostProfile = serde_json::from_str("\"max\"").unwrap();
    assert_eq!(parsed, CostProfile::Max);
}

/// Kimi Code is tiered from its own catalogue, read after `kimi login`. The
/// split is the provider's own naming — "highspeed" is the fast tier — so it
/// is not a guess at which model is stronger.
#[test]
fn kimi_roles_are_tiered_from_its_own_catalogue() {
    assert_eq!(
        CostProfile::Balanced.model_for("kimi", true),
        Some(KIMI_DEEP)
    );
    assert_eq!(
        CostProfile::Balanced.model_for("kimi", false),
        Some(KIMI_FAST)
    );
    assert_eq!(
        CostProfile::Economy.model_for("kimi", true),
        Some(KIMI_FAST),
        "Economy puts every Kimi role on the fast tier"
    );
    assert_eq!(CostProfile::Max.model_for("kimi", false), Some(KIMI_DEEP));
}

/// Kimi has real tiers now, so a profile moves it like Claude rather than
/// merely capping its concurrency.
#[test]
fn a_kimi_agent_is_retiered_and_capped_together() {
    let mut row = record("Scribe Technical Writer", Some("kimi"), None, true);
    row.parallelism = 8;

    assert_eq!(apply_to(&mut row, CostProfile::Economy), Applied::Retiered);
    assert_eq!(row.model.as_deref(), Some(KIMI_FAST));
    assert_eq!(row.parallelism, 1);
}

/// Only ids from Kimi's real catalogue may appear in a tier. One the harness
/// cannot match falls back to the agent default in silence, so an invented id
/// would look applied while changing nothing.
#[test]
fn kimi_tiers_use_only_ids_from_its_real_catalogue() {
    let known = [KIMI_DEEP, KIMI_FAST];
    for profile in [
        CostProfile::Economy,
        CostProfile::Balanced,
        CostProfile::Max,
    ] {
        for deep in [true, false] {
            let model = profile.model_for("kimi", deep).expect("a kimi model");
            assert!(known.contains(&model), "unknown kimi model id {model}");
        }
    }
}

/// An unrecognized runtime is still skipped entirely — being known is what
/// earns a runtime a concurrency cap.
#[test]
fn an_unknown_runtime_is_still_skipped_entirely() {
    let mut row = record("Someone", Some("not-a-runtime"), None, true);
    row.parallelism = 8;
    assert_eq!(apply_to(&mut row, CostProfile::Economy), Applied::Skipped);
    assert_eq!(row.parallelism, 8, "left exactly as it was");
}
