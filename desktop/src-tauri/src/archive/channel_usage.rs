//! Per-channel spend rollup over `agent_metric_index`.
//!
//! The usage series in `agent_usage.rs` answers "how much did this agent
//! spend", with a cumulative/direct ladder and exact-key probes to survive
//! harnesses that report totals rather than deltas. A channel bar asks a much
//! narrower question — what has this one project cost me — so it reads the
//! per-turn columns directly instead of threading a channel filter through
//! that ladder. Keeping the two apart means a channel rollup cannot regress
//! the accounting contract the series owns.
//!
//! Token columns are stored zero-padded for sortability (see
//! `metric_store::encode_u64_sortable`), so every arithmetic use goes through
//! `CAST(... AS INTEGER)`. Rows with `parse_status <> 'valid'` carry no
//! trustworthy numbers: they are excluded from the sums and counted
//! separately, so a gap reads as a gap and never as zero spend.

use rusqlite::{params, Connection};
use serde::Serialize;

/// Spend attributed to one channel over a window.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelUsage {
    /// `None` for the bucket of turns that named no channel.
    pub channel_id: Option<String>,
    pub turns: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// `None` when no turn in the window reported a price — not the same as
    /// a free channel, and the UI must not render it as $0.00.
    pub cost_usd: Option<f64>,
    /// Turns that reported tokens but no price, so `cost_usd` is a floor.
    pub turns_without_cost: i64,
    /// Turns whose payload failed to parse, excluded from every sum above.
    pub unparsed_turns: i64,
    /// Distinct agents that spent in this channel.
    pub agents: i64,
    /// Unix seconds of the most recent turn.
    pub last_turn_at: Option<i64>,
}

/// Cap on returned channel rows, so a pathological store cannot flood the UI.
const MAX_CHANNEL_ROWS: i64 = 500;

/// Roll one window up for a single channel.
///
/// `start` is inclusive and `end` exclusive, matching the series boundaries so
/// a channel bar and the dashboard agree on what "today" means.
pub fn for_channel(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    channel_id: &str,
    start: i64,
    end: i64,
) -> Result<ChannelUsage, String> {
    conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CAST(turn_input_tokens AS INTEGER)), 0),
                COALESCE(SUM(CAST(turn_output_tokens AS INTEGER)), 0),
                SUM(turn_cost_usd),
                COALESCE(SUM(CASE WHEN turn_cost_usd IS NULL THEN 1 ELSE 0 END), 0),
                COUNT(DISTINCT agent_pubkey),
                MAX(event_created_at)
         FROM agent_metric_index
         WHERE identity_pubkey = ?1 AND relay_url = ?2
           AND channel_id = ?3
           AND parse_status = 'valid'
           AND event_created_at >= ?4 AND event_created_at < ?5",
        params![identity_pubkey, relay_url, channel_id, start, end],
        |row| {
            Ok(ChannelUsage {
                channel_id: Some(channel_id.to_string()),
                turns: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                cost_usd: row.get(3)?,
                turns_without_cost: row.get(4)?,
                unparsed_turns: 0,
                agents: row.get(5)?,
                last_turn_at: row.get(6)?,
            })
        },
    )
    .map_err(|error| error.to_string())
    .and_then(|mut usage| {
        usage.unparsed_turns = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_metric_index
                 WHERE identity_pubkey = ?1 AND relay_url = ?2
                   AND channel_id = ?3
                   AND parse_status <> 'valid'
                   AND event_created_at >= ?4 AND event_created_at < ?5",
                params![identity_pubkey, relay_url, channel_id, start, end],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        Ok(usage)
    })
}

/// Roll one window up for every channel that spent in it, dearest first.
///
/// Turns that named no channel are returned as a row with `channel_id: None`
/// rather than dropped, so the channel rows always re-add to the window total
/// and unattributed spend stays visible.
pub fn by_channel(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    start: i64,
    end: i64,
) -> Result<Vec<ChannelUsage>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT channel_id,
                    COUNT(*),
                    COALESCE(SUM(CAST(turn_input_tokens AS INTEGER)), 0),
                    COALESCE(SUM(CAST(turn_output_tokens AS INTEGER)), 0),
                    SUM(turn_cost_usd),
                    COALESCE(SUM(CASE WHEN turn_cost_usd IS NULL THEN 1 ELSE 0 END), 0),
                    COUNT(DISTINCT agent_pubkey),
                    MAX(event_created_at)
             FROM agent_metric_index
             WHERE identity_pubkey = ?1 AND relay_url = ?2
               AND parse_status = 'valid'
               AND event_created_at >= ?3 AND event_created_at < ?4
             GROUP BY channel_id
             ORDER BY COALESCE(SUM(turn_cost_usd), 0) DESC,
                      SUM(CAST(turn_input_tokens AS INTEGER)) DESC
             LIMIT ?5",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(
            params![identity_pubkey, relay_url, start, end, MAX_CHANNEL_ROWS],
            |row| {
                Ok(ChannelUsage {
                    channel_id: row.get(0)?,
                    turns: row.get(1)?,
                    input_tokens: row.get(2)?,
                    output_tokens: row.get(3)?,
                    cost_usd: row.get(4)?,
                    turns_without_cost: row.get(5)?,
                    unparsed_turns: 0,
                    agents: row.get(6)?,
                    last_turn_at: row.get(7)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

/// Roll one window up for a single agent, across every channel.
///
/// The members sidebar asks this per row: an owner scanning a team wants to
/// see which member is doing the spending, next to that member's name. Same
/// window convention as [`for_channel`], so a member bar and the channel bar
/// above it describe the same day.
pub fn for_agent(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    agent_pubkey: &str,
    start: i64,
    end: i64,
) -> Result<ChannelUsage, String> {
    conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CAST(turn_input_tokens AS INTEGER)), 0),
                COALESCE(SUM(CAST(turn_output_tokens AS INTEGER)), 0),
                SUM(turn_cost_usd),
                COALESCE(SUM(CASE WHEN turn_cost_usd IS NULL THEN 1 ELSE 0 END), 0),
                COUNT(DISTINCT channel_id),
                MAX(event_created_at)
         FROM agent_metric_index
         WHERE identity_pubkey = ?1 AND relay_url = ?2
           AND agent_pubkey = ?3
           AND parse_status = 'valid'
           AND event_created_at >= ?4 AND event_created_at < ?5",
        params![identity_pubkey, relay_url, agent_pubkey, start, end],
        |row| {
            Ok(ChannelUsage {
                // Not channel-scoped: this rollup spans every channel the
                // agent worked in, and `agents` carries that channel count.
                channel_id: None,
                turns: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                cost_usd: row.get(3)?,
                turns_without_cost: row.get(4)?,
                unparsed_turns: 0,
                agents: row.get(5)?,
                last_turn_at: row.get(6)?,
            })
        },
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "channel_usage_tests.rs"]
mod tests;

/// The most recent turn each agent took in the window.
///
/// One grouped query rather than one per agent: a settings page listing thirty
/// agents would otherwise open thirty connections to answer a single question.
pub fn last_turn_by_agent(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    start: i64,
    end: i64,
) -> Result<Vec<(String, i64)>, String> {
    let mut statement = conn
        .prepare(
            "SELECT agent_pubkey, MAX(event_created_at)
             FROM agent_metric_index
             WHERE identity_pubkey = ?1 AND relay_url = ?2
               AND event_created_at >= ?3 AND event_created_at < ?4
               AND parse_status = 'valid'
             GROUP BY agent_pubkey",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map(
            rusqlite::params![identity_pubkey, relay_url, start, end],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| error.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// How many turns each agent had that could not be parsed, in the window.
///
/// An unparsed turn is one the agent reported but whose metrics were
/// unreadable — a turn that went wrong. They are excluded from every sum in
/// this module, which is correct for spend and silently wrong for the owner:
/// a stalled agent and a busy one look identical when the only thing shown is
/// what parsed. One grouped query, for the same reason as `last_turn_by_agent`.
pub fn invalid_turns_by_agent(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    start: i64,
    end: i64,
) -> Result<Vec<(String, i64)>, String> {
    let mut statement = conn
        .prepare(
            "SELECT agent_pubkey, COUNT(*)
             FROM agent_metric_index
             WHERE identity_pubkey = ?1 AND relay_url = ?2
               AND event_created_at >= ?3 AND event_created_at < ?4
               AND parse_status <> 'valid'
             GROUP BY agent_pubkey",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map(
            rusqlite::params![identity_pubkey, relay_url, start, end],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| error.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// What each agent was last observed doing: where, on what, and when.
#[derive(Debug, Clone, PartialEq)]
pub struct LastSeen {
    pub agent_pubkey: String,
    pub channel_id: String,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub at: i64,
}

/// The channel each agent worked in most recently, with the harness and model
/// that turn actually ran on.
///
/// Break uses this to record what an agent was doing before it was stopped, so
/// Start can say where it is bringing the agent back to rather than only how
/// many it restarted.
///
/// Harness and model come from here rather than from the agent's stored record
/// on purpose. For an agent linked to a definition, the record's own `model`
/// and runtime are documented as legacy display snapshots — the effective
/// values are resolved from the definition at spawn — so reading them would
/// report the wrong model for exactly the agents that have one. These columns
/// are what the harness itself reported for a turn that really happened.
///
/// The bare columns beside `MAX(event_created_at)` rely on SQLite's documented
/// bare-column behaviour: with a single `MAX` in a `GROUP BY`, unaggregated
/// columns take their values from the row that produced the maximum. That is
/// exactly the row wanted, and it avoids a correlated subquery per agent — but
/// it is a SQLite guarantee, not standard SQL, so it must not be copied into
/// portable code.
///
/// Turns with no channel are excluded rather than reported as a null channel:
/// "worked somewhere unknown" is not something Start can act on, and letting
/// such a row win the `MAX` would hide the most recent channel that is known.
pub fn last_seen_by_agent(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    start: i64,
    end: i64,
) -> Result<Vec<LastSeen>, String> {
    let mut statement = conn
        .prepare(
            "SELECT agent_pubkey, channel_id, harness, model, MAX(event_created_at)
             FROM agent_metric_index
             WHERE identity_pubkey = ?1 AND relay_url = ?2
               AND event_created_at >= ?3 AND event_created_at < ?4
               AND parse_status = 'valid'
               AND channel_id IS NOT NULL
             GROUP BY agent_pubkey",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map(
            rusqlite::params![identity_pubkey, relay_url, start, end],
            |row| {
                Ok(LastSeen {
                    agent_pubkey: row.get(0)?,
                    channel_id: row.get(1)?,
                    harness: row.get(2)?,
                    model: row.get(3)?,
                    at: row.get(4)?,
                })
            },
        )
        .map_err(|error| error.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}
