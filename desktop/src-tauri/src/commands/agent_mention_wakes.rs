//! Recent `@mentions` of this owner's agents, whoever sent them.
//!
//! A mention is meant to wake a stopped agent. That worked only for messages
//! the app itself sent, because the send path checks the mentioned agents
//! before publishing. One agent mentioning another goes straight from the CLI
//! to the relay, and the app never saw it.
//!
//! The home feed cannot fill the gap: it queries `#p` for the owner's own key,
//! so it contains messages that mention *the owner* and nothing else. A
//! request from one specialist to another department — the exact traffic the
//! reporting line produces — is invisible to it.
//!
//! So this asks the relay the question that actually matters: which of my
//! agents were named recently. The caller decides what to do about it, because
//! only the caller knows which of them are already running.

use tauri::State;

use crate::app_state::AppState;
use crate::managed_agents::storage::load_project_launch_records;

/// One agent, and when it was most recently named.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMention {
    pub agent_pubkey: String,
    /// Unix seconds of the earliest message in this window that named it.
    ///
    /// The earliest, not the latest: this becomes a replay floor, and a floor
    /// is a lower bound — starting from the earlier one lets the agent see
    /// every message that asked for it, not just the last.
    pub earliest_at: u64,
}

fn is_cc(tag: &[String]) -> bool {
    tag.get(3).map(String::as_str) == Some("cc")
}

/// Which of this owner's agents were named in channel messages since `since`.
///
/// Copies are excluded. Being kept in the loop is deliberately not a reason to
/// start a process — that is the whole difference between a copy and an ask,
/// and it is what keeps a reporting line from costing a turn per message.
#[tauri::command]
pub async fn recent_agent_mentions(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    since: u64,
) -> Result<Vec<AgentMention>, String> {
    let mine: std::collections::HashSet<String> = load_project_launch_records(&app)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| !record.pubkey.is_empty())
        .map(|record| record.pubkey.to_ascii_lowercase())
        .collect();
    if mine.is_empty() {
        return Ok(Vec::new());
    }

    let filter = serde_json::json!({
        "kinds": [9],
        "#p": mine.iter().collect::<Vec<_>>(),
        "since": since,
        "limit": 200,
    });
    let events = crate::relay::query_relay(&state, &[filter])
        .await
        .unwrap_or_default();

    let mut earliest: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for event in events {
        let created_at = event.created_at.as_secs();
        for tag in event.tags.iter() {
            let parts = tag.as_slice();
            if parts.first().map(String::as_str) != Some("p") || is_cc(parts) {
                continue;
            }
            let Some(pubkey) = parts.get(1).map(|value| value.to_ascii_lowercase()) else {
                continue;
            };
            if !mine.contains(&pubkey) {
                continue;
            }
            earliest
                .entry(pubkey)
                .and_modify(|at| *at = (*at).min(created_at))
                .or_insert(created_at);
        }
    }

    Ok(earliest
        .into_iter()
        .map(|(agent_pubkey, earliest_at)| AgentMention {
            agent_pubkey,
            earliest_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::is_cc;

    fn tag(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| (*p).to_string()).collect()
    }

    /// The marker is the whole difference between an ask and a copy, and it
    /// is what keeps a reporting line from starting a process per message.
    #[test]
    fn a_marked_tag_is_a_copy() {
        assert!(is_cc(&tag(&["p", "aa", "", "cc"])));
    }

    /// An ordinary mention carries no marker, and a relay tag that merely has
    /// a relay hint in the third slot is still an ask.
    #[test]
    fn an_unmarked_tag_is_an_ask() {
        assert!(!is_cc(&tag(&["p", "aa"])));
        assert!(!is_cc(&tag(&["p", "aa", "wss://relay.example"])));
        assert!(!is_cc(&tag(&["p", "aa", "", "reply"])));
    }
}
