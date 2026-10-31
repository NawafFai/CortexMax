//! Owner-facing Break / Start: stop every running agent at once, then bring
//! back exactly the ones that were running.
//!
//! Stopping a team already worked one agent at a time, which is useless in the
//! moment it matters: a run is burning a shared provider limit and the owner
//! wants it to stop *now*. Quitting the app stops the burn too, but it forgets
//! what was running, so resuming means remembering and restarting each pair by
//! hand.
//!
//! Break therefore records the pairs it stops before it stops them, and Start
//! replays that record. The record is a small JSON file beside the agent store,
//! so it survives an app restart and a crash — the two cases where the owner is
//! least able to reconstruct the list themselves.
//!
//! What Break deliberately does not do: kill a turn mid-flight without trace.
//! It reuses the same per-agent stop path the UI already uses, which signals
//! the child, waits for it, writes the stop marker into that agent's log, and
//! removes its runtime receipt. An agent that fails to stop stays tracked and
//! is reported, rather than being dropped from the record and silently left
//! running.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::{
    runtime_commands::{start_managed_agent_runtime, stop_managed_agent_runtime},
    storage::managed_agents_base_dir,
};

/// File holding the pairs a Break stopped, beside `managed-agents.json`.
const RESUME_FILE: &str = "break-resume.json";

/// One agent/community pair that was running when Break ran.
///
/// Everything past `relay_url` is a snapshot of what this agent was doing at
/// the moment it was paused, captured because the answer stops being available
/// once it has stopped: the process is gone, and after an app restart so is
/// any memory of it. Every field added here is `#[serde(default)]`, so a
/// record written before they existed still loads — an upgrade must never turn
/// a pending Break into a file Start refuses to read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PausedPair {
    pub pubkey: String,
    pub relay_url: String,
    /// Display name at the time of the break, so the UI can say who is paused
    /// even if the record is read after a restart.
    #[serde(default)]
    pub name: Option<String>,
    /// Harness its most recent turn actually ran on, as that turn reported it.
    #[serde(default)]
    pub runtime: Option<String>,
    /// Model its most recent turn actually ran on.
    ///
    /// Taken from the archived turn rather than from the agent's record,
    /// because for an agent linked to a definition the record's own `model` is
    /// a legacy display snapshot and the effective model is resolved from the
    /// definition at spawn. It also matters that this is what *ran*: a cost
    /// profile or a failover can retier the team while it is paused, after
    /// which the configuration no longer describes the work being resumed.
    #[serde(default)]
    pub model: Option<String>,
    /// The channel it had most recently taken a turn in.
    #[serde(default)]
    pub last_channel_id: Option<String>,
    /// When that turn was, as a Unix second.
    #[serde(default)]
    pub last_turn_at: Option<i64>,
}

/// The durable record a Break leaves for the next Start.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakRecord {
    /// RFC3339 instant of the break, for "paused 12 minutes ago".
    pub paused_at: Option<String>,
    pub pairs: Vec<PausedPair>,
}

/// Outcome of a Break or a Start, so the UI can report partial success
/// honestly rather than claiming everything stopped.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakOutcome {
    /// Pairs that stopped (Break) or started (Start).
    pub succeeded: usize,
    /// Pairs that refused, with the reason, so none is silently lost.
    pub failures: Vec<PairFailure>,
    /// The record as it now stands on disk.
    pub record: BreakRecord,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairFailure {
    pub pubkey: String,
    pub relay_url: String,
    pub error: String,
}

fn resume_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join(RESUME_FILE))
}

pub(crate) fn read_record(app: &AppHandle) -> Result<BreakRecord, String> {
    let path = resume_path(app)?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            format!("break record is unreadable ({error}); Start cannot restore it")
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BreakRecord::default()),
        Err(error) => Err(error.to_string()),
    }
}

fn write_record(app: &AppHandle, record: &BreakRecord) -> Result<(), String> {
    let path = resume_path(app)?;
    let json = serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?;
    // Same-directory temp + rename, so a crash mid-write leaves either the old
    // record or the new one, never a half-parsed file Start would refuse.
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &json).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

fn clear_record(app: &AppHandle) -> Result<(), String> {
    let path = resume_path(app)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Where each agent was last observed working, keyed by pubkey.
type LastActivity = std::collections::HashMap<String, crate::archive::channel_usage::LastSeen>;

/// Read the last channel each agent took a turn in.
///
/// Best effort on purpose: this enriches the record, so an unreadable archive
/// must degrade the detail Start can report, never prevent a Break. Stopping a
/// team that is burning a provider limit is the urgent half; knowing where it
/// was is the useful half, and the urgent half must not wait on it.
async fn last_activity(app: &AppHandle) -> LastActivity {
    let state = app.state::<crate::app_state::AppState>();
    let Ok(identity) = crate::archive::identity_pubkey(&state) else {
        return LastActivity::new();
    };
    let relay = crate::relay::relay_ws_url_with_override(&state);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    // A week back: far enough that any agent worth resuming has a turn in
    // range, without scanning the whole archive.
    let start = now - 7 * 24 * 3_600;
    state
        .archive_db
        .with_conn(move |conn| {
            crate::archive::channel_usage::last_seen_by_agent(conn, &identity, &relay, start, now)
        })
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|seen| (seen.agent_pubkey.clone(), seen))
                .collect()
        })
        .unwrap_or_default()
}

/// Pairs currently tracked as running, deduplicated and ordered so a Break is
/// deterministic.
fn running_pairs(app: &AppHandle, activity: &LastActivity) -> Result<Vec<PausedPair>, String> {
    let state = app.state::<crate::app_state::AppState>();
    let names: std::collections::HashMap<String, String> = super::load_managed_agents(app)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|record| {
            let name = record.name.clone();
            (!record.pubkey.is_empty()).then_some((record.pubkey, name))
        })
        .collect();
    let runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    let keys: BTreeSet<_> = runtimes
        .keys()
        .map(|key| (key.pubkey.clone(), key.relay_url.clone()))
        .collect();
    Ok(keys
        .into_iter()
        .map(|(pubkey, relay_url)| {
            let seen = activity.get(&pubkey);
            PausedPair {
                name: names.get(&pubkey).cloned(),
                runtime: seen.and_then(|entry| entry.harness.clone()),
                model: seen.and_then(|entry| entry.model.clone()),
                last_channel_id: seen.map(|entry| entry.channel_id.clone()),
                last_turn_at: seen.map(|entry| entry.at),
                pubkey,
                relay_url,
            }
        })
        .collect())
}

/// Stop every running agent and record what was stopped.
///
/// A second Break while a record already exists adds to it rather than
/// replacing it, so an agent started and stopped between two breaks is still
/// restored by the next Start.
#[tauri::command]
pub async fn break_managed_agents(app: AppHandle) -> Result<BreakOutcome, String> {
    // Read before stopping: once the agents are down the archive still holds
    // the history, but doing it first keeps the snapshot describing the moment
    // of the pause rather than a moment after it.
    let activity = last_activity(&app).await;
    tokio::task::spawn_blocking(move || {
        let pairs = running_pairs(&app, &activity)?;
        let mut record = read_record(&app)?;
        let mut outcome = BreakOutcome::default();

        for pair in pairs {
            match stop_managed_agent_runtime(
                pair.pubkey.clone(),
                pair.relay_url.clone(),
                app.clone(),
            ) {
                Ok(_) => {
                    outcome.succeeded += 1;
                    if !record.pairs.iter().any(|existing| {
                        existing.pubkey == pair.pubkey && existing.relay_url == pair.relay_url
                    }) {
                        record.pairs.push(pair);
                    }
                }
                Err(error) => outcome.failures.push(PairFailure {
                    pubkey: pair.pubkey,
                    relay_url: pair.relay_url,
                    error,
                }),
            }
        }

        record.paused_at = Some(crate::util::now_iso());
        // Persist even when nothing stopped, so the UI can show "paused, 0
        // were running" rather than an empty state that reads as "never ran".
        write_record(&app, &record)?;
        outcome.record = record;
        Ok(outcome)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Start exactly the pairs the last Break recorded, then clear the record.
///
/// The record is cleared only when every pair started. A partial resume keeps
/// the failures on disk so a second Start retries them instead of losing them.
#[tauri::command]
pub async fn resume_managed_agents(app: AppHandle) -> Result<BreakOutcome, String> {
    tokio::task::spawn_blocking(move || {
        let record = read_record(&app)?;
        let mut outcome = BreakOutcome::default();
        let mut still_paused = Vec::new();

        for pair in record.pairs.iter().cloned() {
            match start_managed_agent_runtime(
                pair.pubkey.clone(),
                pair.relay_url.clone(),
                app.clone(),
            ) {
                Ok(_) => outcome.succeeded += 1,
                Err(error) => {
                    outcome.failures.push(PairFailure {
                        pubkey: pair.pubkey.clone(),
                        relay_url: pair.relay_url.clone(),
                        error,
                    });
                    still_paused.push(pair);
                }
            }
        }

        if still_paused.is_empty() {
            clear_record(&app)?;
            outcome.record = BreakRecord::default();
        } else {
            let remaining = BreakRecord {
                paused_at: record.paused_at.clone(),
                pairs: still_paused,
            };
            write_record(&app, &remaining)?;
            outcome.record = remaining;
        }
        Ok(outcome)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// The current pause state, for the button's label and count.
#[tauri::command]
pub async fn managed_agents_break_state(app: AppHandle) -> Result<BreakRecord, String> {
    tokio::task::spawn_blocking(move || read_record(&app))
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(test)]
#[path = "break_resume_tests.rs"]
mod tests;
