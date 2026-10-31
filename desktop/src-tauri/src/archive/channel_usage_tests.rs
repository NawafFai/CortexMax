use rusqlite::Connection;

use super::*;

const IDENTITY: &str = "owner";
const RELAY: &str = "wss://relay.example";

/// Minimal `agent_metric_index` with only the columns this rollup reads, so a
/// schema change elsewhere surfaces here as a compile/SQL error rather than a
/// silently wrong total.
fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE agent_metric_index (
             identity_pubkey  TEXT NOT NULL,
             relay_url        TEXT NOT NULL,
             id               TEXT NOT NULL,
             agent_pubkey     TEXT NOT NULL,
             event_created_at INTEGER NOT NULL,
             channel_id       TEXT,
             turn_input_tokens  TEXT,
             turn_output_tokens TEXT,
             turn_cost_usd    REAL,
             parse_status     TEXT NOT NULL,
             PRIMARY KEY (identity_pubkey, relay_url, id)
         );",
    )
    .unwrap();
    conn
}

#[allow(clippy::too_many_arguments)]
fn insert(
    conn: &Connection,
    id: &str,
    agent: &str,
    at: i64,
    channel: Option<&str>,
    input: Option<u64>,
    output: Option<u64>,
    cost: Option<f64>,
    valid: bool,
) {
    conn.execute(
        "INSERT INTO agent_metric_index VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            IDENTITY,
            RELAY,
            id,
            agent,
            at,
            channel,
            input.map(|v| format!("{v:020}")),
            output.map(|v| format!("{v:020}")),
            cost,
            if valid { "valid" } else { "invalid" },
        ],
    )
    .unwrap();
}

#[test]
fn channel_rollup_sums_only_that_channel_inside_the_window() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        100,
        Some("proj"),
        Some(1_000),
        Some(100),
        Some(1.5),
        true,
    );
    insert(
        &conn,
        "b",
        "agent2",
        150,
        Some("proj"),
        Some(500),
        Some(50),
        Some(0.5),
        true,
    );
    // Another channel, and the same channel outside the window: both excluded.
    insert(
        &conn,
        "c",
        "agent1",
        120,
        Some("other"),
        Some(9_999),
        Some(999),
        Some(9.9),
        true,
    );
    insert(
        &conn,
        "d",
        "agent1",
        900,
        Some("proj"),
        Some(7_777),
        Some(777),
        Some(7.7),
        true,
    );

    let usage = for_channel(&conn, IDENTITY, RELAY, "proj", 50, 200).unwrap();
    assert_eq!(usage.turns, 2);
    assert_eq!(usage.input_tokens, 1_500);
    assert_eq!(usage.output_tokens, 150);
    assert_eq!(usage.cost_usd, Some(2.0));
    assert_eq!(usage.agents, 2, "distinct agents, not turns");
    assert_eq!(usage.last_turn_at, Some(150));
}

#[test]
fn the_window_end_is_exclusive_so_adjacent_windows_never_double_count() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        200,
        Some("proj"),
        Some(10),
        Some(1),
        Some(1.0),
        true,
    );
    let before = for_channel(&conn, IDENTITY, RELAY, "proj", 100, 200).unwrap();
    let after = for_channel(&conn, IDENTITY, RELAY, "proj", 200, 300).unwrap();
    assert_eq!(
        before.turns, 0,
        "a turn at the boundary belongs to the later window"
    );
    assert_eq!(after.turns, 1);
}

#[test]
fn a_channel_whose_turns_report_no_price_is_null_not_zero() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        100,
        Some("proj"),
        Some(800),
        Some(80),
        None,
        true,
    );
    insert(
        &conn,
        "b",
        "agent1",
        110,
        Some("proj"),
        Some(200),
        Some(20),
        None,
        true,
    );

    let usage = for_channel(&conn, IDENTITY, RELAY, "proj", 0, 1_000).unwrap();
    assert_eq!(
        usage.cost_usd, None,
        "unknown price must never render as $0.00"
    );
    assert_eq!(usage.turns_without_cost, 2);
    assert_eq!(usage.input_tokens, 1_000, "but the tokens still count");
}

#[test]
fn a_partially_priced_channel_reports_the_floor_and_says_how_many_are_missing() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        100,
        Some("proj"),
        Some(100),
        Some(10),
        Some(2.5),
        true,
    );
    insert(
        &conn,
        "b",
        "agent1",
        110,
        Some("proj"),
        Some(100),
        Some(10),
        None,
        true,
    );

    let usage = for_channel(&conn, IDENTITY, RELAY, "proj", 0, 1_000).unwrap();
    assert_eq!(usage.cost_usd, Some(2.5));
    assert_eq!(
        usage.turns_without_cost, 1,
        "so the UI can mark the figure a floor"
    );
}

#[test]
fn unparsed_turns_are_excluded_from_sums_but_counted() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        100,
        Some("proj"),
        Some(100),
        Some(10),
        Some(1.0),
        true,
    );
    insert(
        &conn,
        "b",
        "agent1",
        110,
        Some("proj"),
        Some(9_999),
        Some(999),
        Some(99.0),
        false,
    );

    let usage = for_channel(&conn, IDENTITY, RELAY, "proj", 0, 1_000).unwrap();
    assert_eq!(usage.turns, 1, "only valid rows are summed");
    assert_eq!(usage.cost_usd, Some(1.0));
    assert_eq!(usage.unparsed_turns, 1, "and the gap is visible");
}

#[test]
fn an_untouched_channel_reports_zeros_rather_than_failing() {
    let conn = db();
    let usage = for_channel(&conn, IDENTITY, RELAY, "never-used", 0, 1_000).unwrap();
    assert_eq!(usage.turns, 0);
    assert_eq!(usage.cost_usd, None);
    assert_eq!(usage.last_turn_at, None);
}

#[test]
fn by_channel_orders_by_spend_and_keeps_unattributed_turns_visible() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        100,
        Some("cheap"),
        Some(100),
        Some(10),
        Some(1.0),
        true,
    );
    insert(
        &conn,
        "b",
        "agent1",
        110,
        Some("dear"),
        Some(100),
        Some(10),
        Some(5.0),
        true,
    );
    insert(
        &conn,
        "c",
        "agent1",
        120,
        None,
        Some(100),
        Some(10),
        Some(3.0),
        true,
    );

    let rows = by_channel(&conn, IDENTITY, RELAY, 0, 1_000).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].channel_id.as_deref(), Some("dear"), "dearest first");
    assert_eq!(
        rows[1].channel_id, None,
        "unattributed spend keeps its place"
    );
    assert_eq!(rows[2].channel_id.as_deref(), Some("cheap"));
    let total: f64 = rows.iter().filter_map(|r| r.cost_usd).sum();
    assert_eq!(total, 9.0, "channel rows re-add to the window total");
}

#[test]
fn rollups_are_scoped_to_one_identity_and_relay() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        100,
        Some("proj"),
        Some(100),
        Some(10),
        Some(1.0),
        true,
    );
    conn.execute(
        "INSERT INTO agent_metric_index VALUES ('other-owner', ?1, 'b', 'agent1', 100, 'proj', \
         '00000000000000009999', '00000000000000000999', 99.0, 'valid')",
        rusqlite::params![RELAY],
    )
    .unwrap();

    let usage = for_channel(&conn, IDENTITY, RELAY, "proj", 0, 1_000).unwrap();
    assert_eq!(
        usage.cost_usd,
        Some(1.0),
        "another owner's spend never leaks in"
    );
}

#[test]
fn agent_rollup_spans_every_channel_that_agent_worked_in() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        100,
        Some("proj"),
        Some(100),
        Some(10),
        Some(1.0),
        true,
    );
    insert(
        &conn,
        "b",
        "agent1",
        110,
        Some("other"),
        Some(200),
        Some(20),
        Some(2.0),
        true,
    );
    // A different agent in the same channels must not leak in.
    insert(
        &conn,
        "c",
        "agent2",
        120,
        Some("proj"),
        Some(900),
        Some(90),
        Some(9.0),
        true,
    );

    let usage = for_agent(&conn, IDENTITY, RELAY, "agent1", 0, 1_000).unwrap();
    assert_eq!(usage.turns, 2);
    assert_eq!(usage.cost_usd, Some(3.0));
    assert_eq!(usage.input_tokens, 300);
    assert_eq!(usage.agents, 2, "counts the channels this agent spent in");
    assert_eq!(usage.channel_id, None, "the rollup is not channel-scoped");
}

#[test]
fn an_agent_with_no_turns_today_reports_zeros() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        5_000,
        Some("proj"),
        Some(100),
        Some(10),
        Some(1.0),
        true,
    );
    let usage = for_agent(&conn, IDENTITY, RELAY, "agent1", 0, 1_000).unwrap();
    assert_eq!(usage.turns, 0);
    assert_eq!(
        usage.cost_usd, None,
        "outside the window is unknown, not zero"
    );
}

#[test]
fn the_last_turn_per_agent_is_one_query_over_the_whole_team() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        100,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        true,
    );
    insert(
        &conn,
        "b",
        "agent1",
        500,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        true,
    );
    insert(
        &conn,
        "c",
        "agent2",
        300,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        true,
    );

    let rows = last_turn_by_agent(&conn, IDENTITY, RELAY, 0, 1_000).unwrap();
    let found: std::collections::BTreeMap<_, _> = rows.into_iter().collect();
    assert_eq!(
        found.get("agent1"),
        Some(&500),
        "the latest turn, not the first"
    );
    assert_eq!(found.get("agent2"), Some(&300));
}

#[test]
fn an_agent_whose_only_turn_is_outside_the_window_is_absent() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        5_000,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        true,
    );
    let rows = last_turn_by_agent(&conn, IDENTITY, RELAY, 0, 1_000).unwrap();
    assert!(rows.is_empty(), "outside the window is not a last turn");
}

/// An unparsed row is not evidence the agent worked, so it must not become
/// that agent's "last turn" and make a stalled agent look busy.
#[test]
fn an_unparsed_turn_is_not_counted_as_the_last_turn() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        100,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        true,
    );
    insert(
        &conn,
        "b",
        "agent1",
        900,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        false,
    );

    let rows = last_turn_by_agent(&conn, IDENTITY, RELAY, 0, 1_000).unwrap();
    assert_eq!(rows[0].1, 100, "the invalid later row must not win");
}

/// A turn that could not be parsed is a turn that went wrong. It is excluded
/// from every sum here, which is right for spend and silently wrong for the
/// owner: a stalled agent looks identical to a busy one when only what parsed
/// is shown.
#[test]
fn failed_turns_are_counted_per_agent() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        100,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        true,
    );
    insert(
        &conn,
        "b",
        "agent1",
        110,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        false,
    );
    insert(
        &conn,
        "c",
        "agent1",
        120,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        false,
    );
    insert(
        &conn,
        "d",
        "agent2",
        130,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        true,
    );

    let rows = invalid_turns_by_agent(&conn, IDENTITY, RELAY, 0, 1_000).unwrap();
    let found: std::collections::BTreeMap<_, _> = rows.into_iter().collect();
    assert_eq!(found.get("agent1"), Some(&2));
    assert_eq!(
        found.get("agent2"),
        None,
        "an agent with no failures must not appear at all, rather than as a zero"
    );
}

#[test]
fn failed_turns_outside_the_window_are_not_counted() {
    let conn = db();
    insert(
        &conn,
        "a",
        "agent1",
        5_000,
        Some("proj"),
        Some(1),
        Some(1),
        Some(1.0),
        false,
    );
    assert!(invalid_turns_by_agent(&conn, IDENTITY, RELAY, 0, 1_000)
        .unwrap()
        .is_empty());
}

/// Another owner's failures are not this owner's problem.
#[test]
fn failed_turns_are_scoped_to_one_identity() {
    let conn = db();
    conn.execute(
        "INSERT INTO agent_metric_index VALUES ('other-owner', ?1, 'x', 'agent1', 100, 'proj', \
         NULL, NULL, NULL, 'invalid')",
        rusqlite::params![RELAY],
    )
    .unwrap();
    assert!(invalid_turns_by_agent(&conn, IDENTITY, RELAY, 0, 1_000)
        .unwrap()
        .is_empty());
}
