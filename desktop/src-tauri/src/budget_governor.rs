//! The governor: act on a provider limit before it stops the team for you.
//!
//! Knowing a window is at 99% only helps if someone is looking. A run that
//! starts at midnight and exhausts a weekly cap by morning is not prevented by
//! a bar in the sidebar — by the time it is read, the week is gone. The
//! governor watches the same readings the panel shows and takes one chosen
//! action when a window crosses the owner's threshold.
//!
//! Three properties matter more than the feature itself:
//!
//! * **Off by default.** Stopping someone's team, or moving it to cheaper
//!   models, is their decision. The governor does nothing until it is switched
//!   on and given an action.
//! * **Once per window.** A crossing fires a single time. If the owner presses
//!   Start after an automatic Break, the governor does not immediately break
//!   them again — the window is already recorded as handled, and only a new
//!   window (a different reset time) can fire again.
//! * **Never resumes.** A limit clearing does not mean the owner wants the
//!   team running. Start stays a human action.
//!
//! A stale reading can never fire. Codex leaves its last figure behind when it
//! goes idle, so a window that has already reset still reads 99% until Codex
//! runs again; acting on that would break a team over a limit that no longer
//! exists. [`crate::provider_limits`] marks those, and [`decide`] skips them.

use serde::{Deserialize, Serialize};

use crate::provider_limits::LimitGroup;

/// What the governor does when a window crosses the threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum GovernorAction {
    /// Say so and change nothing.
    #[default]
    Notify,
    /// Move the whole team to the cheapest model tier.
    Economy,
    /// Stop every running agent, recording them so Start can bring them back.
    Break,
}

/// Owner settings. `enabled` is false until they turn it on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernorConfig {
    pub enabled: bool,
    /// Percent of a window at which to act.
    pub threshold_percent: f64,
    pub action: GovernorAction,
}

impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            // Late enough not to nag, early enough to leave room to react.
            threshold_percent: 90.0,
            action: GovernorAction::Notify,
        }
    }
}

/// A crossing worth acting on.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    /// Identifies the specific window occurrence, so one crossing fires once.
    pub window_key: String,
    /// Owner-facing explanation, naming the runtime, window and figure.
    pub reason: String,
    pub runtime: String,
    pub used_percent: f64,
}

/// Identify one occurrence of a window.
///
/// The reset time is part of the key: next week's window is a different
/// occurrence of the same named limit, and must be allowed to fire again.
fn window_key(group: &LimitGroup, label: &str, resets: &str) -> String {
    format!("{}:{}:{}", group.runtime, label, resets)
}

/// The crossing to act on, if any.
///
/// `handled` holds the keys already acted on. Returns the fullest unhandled
/// window at or above the threshold, so the most urgent one is reported when
/// several cross at once.
pub fn decide(
    config: &GovernorConfig,
    groups: &[LimitGroup],
    handled: &[String],
) -> Option<Trigger> {
    if !config.enabled {
        return None;
    }

    groups
        .iter()
        .flat_map(|group| group.windows.iter().map(move |window| (group, window)))
        // A window that has already reset says nothing about now; acting on it
        // would stop a team over a limit that no longer exists.
        .filter(|(_, window)| !window.stale)
        .filter(|(_, window)| window.used_percent >= config.threshold_percent)
        .map(|(group, window)| {
            let resets = window
                .resets_at
                .map(|at| at.to_string())
                .or_else(|| window.resets.clone())
                .unwrap_or_else(|| "no-reset".to_string());
            Trigger {
                window_key: window_key(group, &window.label, &resets),
                reason: format!(
                    "{} · {} is at {:.0}%{}",
                    group.label,
                    window.label,
                    window.used_percent,
                    window
                        .resets
                        .as_ref()
                        .map(|when| format!(", resets {when}"))
                        .unwrap_or_default()
                ),
                runtime: group.runtime.clone(),
                used_percent: window.used_percent,
            }
        })
        .filter(|trigger| !handled.iter().any(|key| key == &trigger.window_key))
        .max_by(|left, right| {
            left.used_percent
                .partial_cmp(&right.used_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

// ---------------------------------------------------------------------------
// Persistence and the tick
// ---------------------------------------------------------------------------

use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

use crate::managed_agents::storage::managed_agents_base_dir;

const GOVERNOR_FILE: &str = "budget-governor.json";
/// How often the governor checks. The limit readings behind it move over
/// minutes at best, and the Claude read is cached, so a tighter loop would add
/// no warning and cost requests.
const TICK: std::time::Duration = std::time::Duration::from_secs(120);

/// What the governor has done, for the panel to show and for the once-per-
/// window rule to consult.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GovernorRecord {
    pub config: GovernorConfig,
    /// Window occurrences already acted on.
    pub handled: Vec<String>,
    pub last_fired_at: Option<String>,
    pub last_reason: Option<String>,
    pub last_action: Option<GovernorAction>,
    /// What the action actually did, in the action's own words.
    pub last_outcome: Option<String>,
}

fn record_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join(GOVERNOR_FILE))
}

fn read_record(app: &AppHandle) -> Result<GovernorRecord, String> {
    let path = record_path(app)?;
    match std::fs::read(&path) {
        // No file is the normal first-run state, not a failure.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(GovernorRecord::default()),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
        Ok(bytes) => serde_json::from_slice(&bytes)
            // A corrupt record must not disable the governor's settings screen;
            // defaults are safe because the default is "off".
            .or_else(|_| Ok(GovernorRecord::default())),
    }
}

fn write_record(app: &AppHandle, record: &GovernorRecord) -> Result<(), String> {
    let path = record_path(app)?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?;
    std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

/// The owner's current governor settings and history.
#[tauri::command]
pub async fn budget_governor_state(app: AppHandle) -> Result<GovernorRecord, String> {
    tokio::task::spawn_blocking(move || read_record(&app))
        .await
        .map_err(|error| error.to_string())?
}

/// Replace the settings, keeping the history.
///
/// Changing the settings clears `handled`: an owner who lowers the threshold or
/// picks a stronger action is asking for it to apply to the window they are in
/// now, not only to the next one.
#[tauri::command]
pub async fn set_budget_governor(
    config: GovernorConfig,
    app: AppHandle,
) -> Result<GovernorRecord, String> {
    tokio::task::spawn_blocking(move || {
        let mut record = read_record(&app)?;
        if record.config != config {
            record.handled.clear();
        }
        record.config = config;
        write_record(&app, &record)?;
        Ok(record)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Run the chosen action. Returns what it did, in plain words.
async fn perform(action: GovernorAction, app: &AppHandle) -> Result<String, String> {
    match action {
        GovernorAction::Notify => Ok("notified only; nothing was changed".to_string()),
        GovernorAction::Economy => {
            let outcome = crate::managed_agents::cost_profile::apply_cost_profile(
                crate::managed_agents::cost_profile::CostProfile::Economy,
                app.clone(),
            )
            .await?;
            Ok(format!(
                "moved the team to Economy: {} retiered, {} capped, {} already set",
                outcome.retiered, outcome.capped_only, outcome.unchanged
            ))
        }
        GovernorAction::Break => {
            let outcome =
                crate::managed_agents::break_resume::break_managed_agents(app.clone()).await?;
            let failed = outcome.failures.len();
            Ok(format!(
                "stopped {} agent{}{}. Press Start when you are ready — the governor never resumes for you",
                outcome.succeeded,
                if outcome.succeeded == 1 { "" } else { "s" },
                if failed == 0 {
                    String::new()
                } else {
                    format!(", {failed} refused to stop")
                }
            ))
        }
    }
}

/// One governor pass: read the limits, decide, act, record.
///
/// Errors are returned rather than logged away, but the caller treats them as
/// a skipped tick: a provider that cannot be read right now is not a reason to
/// stop checking for the rest of the session.
pub async fn tick(app: &AppHandle) -> Result<Option<Trigger>, String> {
    let record = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || read_record(&app))
            .await
            .map_err(|error| error.to_string())??
    };
    if !record.config.enabled {
        return Ok(None);
    }

    let groups = crate::provider_limits::get_provider_limits(None).await?;
    let Some(trigger) = decide(&record.config, &groups, &record.handled) else {
        return Ok(None);
    };

    let outcome = perform(record.config.action, app).await?;

    let app_for_write = app.clone();
    let trigger_for_write = trigger.clone();
    let action = record.config.action;
    tokio::task::spawn_blocking(move || {
        // Re-read rather than reusing the copy above: the owner may have
        // changed the settings while the action ran, and their change wins.
        let mut latest = read_record(&app_for_write)?;
        latest.handled.push(trigger_for_write.window_key.clone());
        latest.last_fired_at = Some(crate::util::now_iso());
        latest.last_reason = Some(trigger_for_write.reason.clone());
        latest.last_action = Some(action);
        latest.last_outcome = Some(outcome);
        write_record(&app_for_write, &latest)
    })
    .await
    .map_err(|error| error.to_string())??;

    // The owner must never discover an automatic Break by finding their team
    // stopped; the UI listens for this and says what happened.
    let _ = app.emit("budget-governor-fired", &trigger);
    Ok(Some(trigger))
}

/// Start the governor's background loop for the life of the app.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        loop {
            interval.tick().await;
            if let Err(error) = tick(&app).await {
                eprintln!("budget-governor: skipped a check: {error}");
            }
        }
    });
}

#[cfg(test)]
#[path = "budget_governor_tests.rs"]
mod tests;
