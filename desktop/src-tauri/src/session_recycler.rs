//! Recycle long-lived agent sessions, so a day's work does not cost more each
//! hour than it did the last.
//!
//! An ACP session accumulates. Every turn re-sends the conversation so far, so
//! an agent that has been up since morning pays for the whole morning on each
//! afternoon turn — the cost of the same piece of work grows with the age of
//! the session it happens in. A real team's usage report was explicit
//! about it: 67% of usage came from sessions active 8+ hours, and 86% was at
//! over 150k of context.
//!
//! Restarting an agent gives it a fresh session, which drops its context back
//! to the system prompt. The work the team has done is not lost — it lives in
//! the channel, which is where agents read their history from.
//!
//! The danger is obvious and is what most of this module is about: restarting
//! an agent mid-turn throws away the turn. So a session is only recycled when
//! it is both **old** and **quiet** — nothing has come out of it for a while.
//! Quiet is measured from the agent's last recorded turn, not from a guess.
//! An agent whose last turn cannot be read is left alone, because "no data"
//! and "idle" are not the same thing.

use serde::{Deserialize, Serialize};

/// Owner settings. Disabled until switched on: restarting someone's agents is
/// their decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecyclerConfig {
    pub enabled: bool,
    /// Recycle a session once it has been up this long.
    pub max_session_hours: f64,
    /// ...but only after this long with no turn, so work is never cut off.
    pub min_idle_minutes: f64,
}

impl Default for RecyclerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            // A full working session, matching the "8+ hours" band the usage
            // report singles out.
            max_session_hours: 8.0,
            // Long enough that a thinking agent is not mistaken for a finished
            // one; short enough to catch a genuinely parked session.
            min_idle_minutes: 20.0,
        }
    }
}

/// One agent considered for recycling.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub pubkey: String,
    pub relay_url: String,
    pub name: String,
    /// Unix seconds the current session started, from the agent's record.
    pub started_at: Option<i64>,
    /// Unix seconds of the agent's last recorded turn.
    pub last_turn_at: Option<i64>,
}

/// Why an agent was or was not recycled — the UI reports this rather than a
/// bare count, so an owner can see the rule working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Old enough and quiet enough.
    Recycle,
    /// The session has not been up long enough yet.
    TooYoung,
    /// The agent is still working; cutting in would lose the turn.
    Busy,
    /// Nothing is known about when it last worked, so it is left alone.
    Unknown,
}

/// Decide what to do with one agent.
///
/// `now` is unix seconds, passed in so the rule is testable without waiting
/// eight hours for a session to age.
pub fn verdict(config: &RecyclerConfig, candidate: &Candidate, now: i64) -> Verdict {
    if !config.enabled {
        return Verdict::TooYoung;
    }
    let Some(started_at) = candidate.started_at else {
        // An agent with no recorded start is usually one adopted from a
        // previous run. Its session age is genuinely unknown.
        return Verdict::Unknown;
    };

    let age_hours = (now - started_at) as f64 / 3_600.0;
    if age_hours < config.max_session_hours {
        return Verdict::TooYoung;
    }

    // Age alone is not enough: a session that is old *and* still producing
    // turns is doing exactly what it should.
    let Some(last_turn_at) = candidate.last_turn_at else {
        return Verdict::Unknown;
    };
    let idle_minutes = (now - last_turn_at) as f64 / 60.0;
    if idle_minutes < config.min_idle_minutes {
        return Verdict::Busy;
    }
    Verdict::Recycle
}

/// What one pass did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecycleOutcome {
    pub recycled: usize,
    pub skipped_busy: usize,
    pub skipped_young: usize,
    pub skipped_unknown: usize,
    /// Names of the agents actually restarted.
    pub names: Vec<String>,
    /// Agents whose restart failed, with the reason.
    pub failures: Vec<String>,
}

// ---------------------------------------------------------------------------
// Persistence and the pass
// ---------------------------------------------------------------------------

use std::collections::BTreeSet;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use crate::managed_agents::storage::managed_agents_base_dir;

const RECYCLER_FILE: &str = "session-recycler.json";
/// A session ages over hours, so checking every ten minutes is frequent enough
/// to act promptly and rare enough to cost nothing.
const TICK: std::time::Duration = std::time::Duration::from_secs(600);

fn record_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join(RECYCLER_FILE))
}

fn read_config(app: &AppHandle) -> Result<RecyclerConfig, String> {
    match std::fs::read(record_path(app)?) {
        // No file is the normal first-run state.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(RecyclerConfig::default()),
        Err(error) => Err(error.to_string()),
        // A corrupt file falls back to the defaults, which are safe because
        // the default is "off".
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
    }
}

fn write_config(app: &AppHandle, config: &RecyclerConfig) -> Result<(), String> {
    let path = record_path(app)?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

/// The owner's recycler settings.
#[tauri::command]
pub async fn session_recycler_config(app: AppHandle) -> Result<RecyclerConfig, String> {
    tokio::task::spawn_blocking(move || read_config(&app))
        .await
        .map_err(|error| error.to_string())?
}

/// Replace the recycler settings.
#[tauri::command]
pub async fn set_session_recycler(
    config: RecyclerConfig,
    app: AppHandle,
) -> Result<RecyclerConfig, String> {
    tokio::task::spawn_blocking(move || {
        write_config(&app, &config)?;
        Ok(config)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// The agents with a live runtime, paired with when their session started.
///
/// Only running agents are considered: a stopped agent has no session to
/// recycle, and restarting it would start work nobody asked for.
fn running_candidates(app: &AppHandle) -> Result<Vec<Candidate>, String> {
    let state = app.state::<crate::app_state::AppState>();
    let live: BTreeSet<(String, String)> = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?
        .keys()
        .map(|key| (key.pubkey.clone(), key.relay_url.clone()))
        .collect();

    Ok(crate::managed_agents::load_managed_agents(app)?
        .into_iter()
        .filter(|record| live.contains(&(record.pubkey.clone(), record.relay_url.clone())))
        .map(|record| Candidate {
            started_at: record
                .last_started_at
                .as_deref()
                .and_then(|when| chrono::DateTime::parse_from_rfc3339(when).ok())
                .map(|when| when.timestamp()),
            pubkey: record.pubkey,
            relay_url: record.relay_url,
            name: record.name,
            // Filled in from the archive below; the store does not record it.
            last_turn_at: None,
        })
        .collect())
}

/// One recycler pass.
pub async fn tick(app: &AppHandle) -> Result<RecycleOutcome, String> {
    let (config, candidates) = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            Ok::<_, String>((read_config(&app)?, running_candidates(&app)?))
        })
        .await
        .map_err(|error| error.to_string())??
    };

    let mut outcome = RecycleOutcome::default();
    if !config.enabled || candidates.is_empty() {
        return Ok(outcome);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);

    for mut candidate in candidates {
        // Only ask the archive for agents that are already old enough; the
        // query is per-agent and most of them will not be.
        if verdict(&config, &candidate, now) == Verdict::TooYoung {
            outcome.skipped_young += 1;
            continue;
        }
        candidate.last_turn_at = last_turn_at(app, &candidate.pubkey, now).await;

        match verdict(&config, &candidate, now) {
            Verdict::Recycle => {
                let restart = crate::managed_agents::restart_managed_agent_runtime(
                    candidate.pubkey.clone(),
                    candidate.relay_url.clone(),
                    app.clone(),
                );
                match restart {
                    Ok(_) => {
                        outcome.recycled += 1;
                        outcome.names.push(candidate.name.clone());
                    }
                    // A refused restart is reported, never swallowed: the agent
                    // is still running on its old session and the owner should
                    // know the rule did not take effect.
                    Err(error) => outcome
                        .failures
                        .push(format!("{}: {error}", candidate.name)),
                }
            }
            Verdict::Busy => outcome.skipped_busy += 1,
            Verdict::TooYoung => outcome.skipped_young += 1,
            Verdict::Unknown => outcome.skipped_unknown += 1,
        }
    }
    Ok(outcome)
}

/// When this agent last took a turn, from the archived metrics.
///
/// `None` on any failure, which [`verdict`] treats as "unknown" and therefore
/// never recycles — an unreadable archive must not become a reason to restart
/// an agent that might be mid-turn.
pub(crate) async fn last_turn_at(app: &AppHandle, agent_pubkey: &str, now: i64) -> Option<i64> {
    let state = app.state::<crate::app_state::AppState>();
    let identity = crate::archive::identity_pubkey(&state).ok()?;
    let relay = crate::relay::relay_ws_url_with_override(&state);
    let agent = agent_pubkey.to_string();
    // A week back is far enough that any agent worth recycling has a turn in
    // range, without scanning the whole archive.
    let start = now - 7 * 24 * 3_600;
    state
        .archive_db
        .with_conn(move |conn| {
            crate::archive::channel_usage::for_agent(conn, &identity, &relay, &agent, start, now)
        })
        .await
        .ok()?
        .last_turn_at
}

/// Start the recycler's background loop for the life of the app.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        loop {
            interval.tick().await;
            match tick(&app).await {
                Ok(outcome) if outcome.recycled > 0 => {
                    eprintln!(
                        "session-recycler: recycled {} session(s): {}",
                        outcome.recycled,
                        outcome.names.join(", ")
                    );
                }
                Ok(_) => {}
                Err(error) => eprintln!("session-recycler: skipped a pass: {error}"),
            }
        }
    });
}

#[cfg(test)]
#[path = "session_recycler_tests.rs"]
mod tests;
