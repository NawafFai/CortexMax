use super::*;

fn pair(pubkey: &str, relay: &str) -> PausedPair {
    PausedPair {
        pubkey: pubkey.to_string(),
        relay_url: relay.to_string(),
        name: Some(format!("agent {pubkey}")),
        runtime: Some("codex-acp".to_string()),
        model: Some("gpt-5.6-terra".to_string()),
        last_channel_id: Some("channel-1".to_string()),
        last_turn_at: Some(1_750_000_000),
    }
}

/// The record is the whole point of Break: it is what a Start replays after a
/// quit or a crash, so it must survive a round trip through JSON unchanged.
#[test]
fn a_break_record_round_trips_through_its_file_format() {
    let record = BreakRecord {
        paused_at: Some("2026-09-19T10:00:00Z".to_string()),
        pairs: vec![pair("a", "wss://one.test"), pair("b", "wss://two.test")],
    };
    let json = serde_json::to_vec_pretty(&record).unwrap();
    let parsed: BreakRecord = serde_json::from_slice(&json).unwrap();
    assert_eq!(parsed, record);
}

/// An older record written before names were stored must still load, rather
/// than failing and stranding every paused agent.
#[test]
fn a_record_without_names_still_loads() {
    let json = br#"{"pausedAt":"2026-09-19T10:00:00Z","pairs":[{"pubkey":"a","relayUrl":"wss://one.test"}]}"#;
    let parsed: BreakRecord = serde_json::from_slice(json).unwrap();
    assert_eq!(parsed.pairs.len(), 1);
    assert_eq!(parsed.pairs[0].name, None);
}

/// An absent file means "nothing is paused", which is a normal state, not an
/// error — the UI asks for this on every open.
#[test]
fn an_empty_record_is_the_default_not_a_failure() {
    let record = BreakRecord::default();
    assert!(record.pairs.is_empty());
    assert_eq!(record.paused_at, None);
}

/// Serialization uses camelCase so the TypeScript side reads it without a
/// translation layer; a rename here would silently break the buttons.
#[test]
fn the_wire_shape_matches_what_the_ui_reads() {
    let record = BreakRecord {
        paused_at: Some("2026-09-19T10:00:00Z".to_string()),
        pairs: vec![pair("a", "wss://one.test")],
    };
    let json = serde_json::to_string(&record).unwrap();
    assert!(json.contains("\"pausedAt\""), "{json}");
    assert!(json.contains("\"relayUrl\""), "{json}");
    assert!(!json.contains("\"paused_at\""), "{json}");
}

/// A Break that stopped nothing still reports its record, so the UI can say
/// "paused, nothing was running" instead of showing a never-used empty state.
#[test]
fn an_outcome_carries_both_successes_and_failures() {
    let outcome = BreakOutcome {
        succeeded: 2,
        failures: vec![PairFailure {
            pubkey: "c".to_string(),
            relay_url: "wss://three.test".to_string(),
            error: "child would not exit".to_string(),
        }],
        record: BreakRecord {
            paused_at: Some("2026-09-19T10:00:00Z".to_string()),
            pairs: vec![pair("a", "wss://one.test"), pair("b", "wss://two.test")],
        },
    };
    let json = serde_json::to_string(&outcome).unwrap();
    assert!(json.contains("\"succeeded\":2"), "{json}");
    assert!(
        json.contains("child would not exit"),
        "a refused stop must reach the owner, not vanish: {json}"
    );
}

/// Pairs are ordered and deduplicated by (pubkey, relay), so breaking twice
/// cannot enter the same agent twice and Start cannot double-launch it.
#[test]
fn pairs_are_comparable_so_the_record_can_dedupe_by_identity() {
    let one = pair("a", "wss://one.test");
    let same_pair_other_name = PausedPair {
        name: Some("renamed".to_string()),
        ..one.clone()
    };
    assert_eq!(one.pubkey, same_pair_other_name.pubkey);
    assert_eq!(one.relay_url, same_pair_other_name.relay_url);

    let mut pairs = [
        pair("b", "wss://two.test"),
        pair("a", "wss://one.test"),
        pair("a", "wss://two.test"),
    ];
    pairs.sort();
    assert_eq!(
        pairs
            .iter()
            .map(|p| (p.pubkey.as_str(), p.relay_url.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("a", "wss://one.test"),
            ("a", "wss://two.test"),
            ("b", "wss://two.test"),
        ],
        "same agent on two communities stays two distinct pairs"
    );
}

/// A record written by a build that predates the activity snapshot must still
/// load. Losing it would mean a pending Break becomes unrestorable across an
/// upgrade — the one moment the record exists to survive.
#[test]
fn a_record_from_before_the_snapshot_fields_still_loads() {
    let old = r#"{
      "pausedAt": "2026-09-19T10:00:00Z",
      "pairs": [{ "pubkey": "a", "relayUrl": "wss://one.test", "name": "Sara" }]
    }"#;
    let parsed: BreakRecord = serde_json::from_str(old).expect("old records must still parse");
    assert_eq!(parsed.pairs.len(), 1);
    let pair = &parsed.pairs[0];
    assert_eq!(pair.name.as_deref(), Some("Sara"));
    assert_eq!(pair.runtime, None, "absent is absent, not a guess");
    assert_eq!(pair.model, None);
    assert_eq!(pair.last_channel_id, None);
    assert_eq!(pair.last_turn_at, None);
}

/// The snapshot is what Start reports, so it has to survive the file too.
#[test]
fn the_activity_snapshot_round_trips_with_the_record() {
    let record = BreakRecord {
        paused_at: Some("2026-09-19T10:00:00Z".to_string()),
        pairs: vec![pair("a", "wss://one.test")],
    };
    let json = serde_json::to_vec_pretty(&record).unwrap();
    let parsed: BreakRecord = serde_json::from_slice(&json).unwrap();
    assert_eq!(parsed, record);
    let restored = &parsed.pairs[0];
    assert_eq!(restored.last_channel_id.as_deref(), Some("channel-1"));
    assert_eq!(restored.model.as_deref(), Some("gpt-5.6-terra"));
}

/// An agent with no archived turn is still paused and still resumed; it simply
/// has nothing to say about where it was. Dropping it from the record to avoid
/// an empty snapshot would leave it stopped after the next Start.
#[test]
fn a_pair_with_no_observed_activity_is_still_a_valid_record_entry() {
    let bare = PausedPair {
        pubkey: "a".to_string(),
        relay_url: "wss://one.test".to_string(),
        name: Some("Sara".to_string()),
        runtime: None,
        model: None,
        last_channel_id: None,
        last_turn_at: None,
    };
    let record = BreakRecord {
        paused_at: Some("2026-09-19T10:00:00Z".to_string()),
        pairs: vec![bare.clone()],
    };
    let json = serde_json::to_vec_pretty(&record).unwrap();
    let parsed: BreakRecord = serde_json::from_slice(&json).unwrap();
    assert_eq!(parsed.pairs, vec![bare]);
}
