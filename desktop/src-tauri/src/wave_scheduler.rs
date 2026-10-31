//! Run the team a wave at a time instead of all at once.
//!
//! Every agent that is mentioned wakes immediately, so a thirty-agent team
//! becomes thirty concurrent sessions competing for one shared provider
//! window. They do not go faster for it: the window is the bottleneck, and
//! crowding it means every agent's turn is slower and the limit arrives
//! sooner. One real team's usage report put a number on the effect — 42% of
//! usage happened while four or more sessions ran in parallel.
//!
//! A wave caps how many agents are *running* at once. The rest are stopped,
//! not deleted: they keep their identity, their channel membership and their
//! history, and they are started as places free up.
//!
//! Two rules decide who runs, in this order:
//!
//! 1. **Coordination first.** The roles that decide what everyone else does
//!    are useless when queued behind the work they are meant to direct, so
//!    they hold a place in every wave.
//! 2. **Then longest-waiting.** Among the rest, whoever has gone longest
//!    without a turn goes next, so a queue cannot starve one agent forever.
//!
//! Nothing is ever stopped mid-turn. An agent that is working keeps its place
//! even if a higher-priority agent is waiting, because cutting a turn off
//! wastes everything that turn had already spent — which is the very thing
//! this module exists to avoid.

use serde::{Deserialize, Serialize};

/// Owner settings. Off until switched on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaveConfig {
    pub enabled: bool,
    /// How many agents may be running at once.
    pub concurrent_agents: usize,
    /// An agent with no turn for this long is treated as finished, and its
    /// place may be given away.
    pub idle_minutes: f64,
}

impl Default for WaveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            // Four is where the usage report's parallelism penalty begins.
            concurrent_agents: 4,
            // Long enough not to mistake a thinking agent for a finished one.
            idle_minutes: 10.0,
        }
    }
}

/// One agent the scheduler can act on.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveAgent {
    pub pubkey: String,
    pub relay_url: String,
    pub name: String,
    /// Whether it currently has a runtime.
    pub running: bool,
    /// Unix seconds of its last recorded turn, if any.
    pub last_turn_at: Option<i64>,
}

/// What the scheduler wants done this pass.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WavePlan {
    /// Agents to stop, because the wave is over its cap.
    pub stop: Vec<String>,
    /// Agents to start, because a place is free.
    pub start: Vec<String>,
    /// Running agents left alone because they are mid-turn.
    pub held_busy: usize,
}

/// Whether this agent's turn is still in flight.
///
/// An agent with no recorded turn at all is treated as busy, not idle: it may
/// be working on its first turn, and stopping it would throw that away.
fn is_busy(agent: &WaveAgent, config: &WaveConfig, now: i64) -> bool {
    match agent.last_turn_at {
        Some(last) => ((now - last) as f64 / 60.0) < config.idle_minutes,
        None => true,
    }
}

/// How long this agent has been waiting for a turn, for queue ordering.
///
/// An agent that has never taken a turn has waited longest of all — it has
/// been waiting since the team started — so it sorts ahead of one that ran an
/// hour ago rather than behind it.
fn waited_since(agent: &WaveAgent) -> i64 {
    agent.last_turn_at.unwrap_or(i64::MIN)
}

/// Decide the next wave.
///
/// `is_lead` marks the coordinating roles that hold a place in every wave.
pub fn plan(
    config: &WaveConfig,
    agents: &[WaveAgent],
    now: i64,
    is_lead: &dyn Fn(&str) -> bool,
) -> WavePlan {
    let mut out = WavePlan::default();
    if !config.enabled || config.concurrent_agents == 0 {
        return out;
    }

    // Priority order: leads first, then whoever has waited longest.
    let mut ordered: Vec<&WaveAgent> = agents.iter().collect();
    ordered.sort_by(|left, right| {
        is_lead(&right.name)
            .cmp(&is_lead(&left.name))
            .then_with(|| waited_since(left).cmp(&waited_since(right)))
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut places = config.concurrent_agents;
    let mut wanted: Vec<&WaveAgent> = Vec::new();
    for agent in &ordered {
        if places == 0 {
            break;
        }
        wanted.push(agent);
        places -= 1;
    }

    for agent in agents {
        let chosen = wanted.iter().any(|pick| pick.pubkey == agent.pubkey);
        match (agent.running, chosen) {
            // Over the cap. A busy agent keeps its place regardless: cutting a
            // turn off wastes everything that turn already spent.
            (true, false) => {
                if is_busy(agent, config, now) {
                    out.held_busy += 1;
                } else {
                    out.stop.push(agent.pubkey.clone());
                }
            }
            (false, true) => out.start.push(agent.pubkey.clone()),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Persistence and the pass
// ---------------------------------------------------------------------------

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use crate::managed_agents::storage::managed_agents_base_dir;

const WAVE_FILE: &str = "wave-scheduler.json";
/// Long enough that an agent finishing a turn is not immediately replaced,
/// short enough that a freed place does not sit empty.
const TICK: std::time::Duration = std::time::Duration::from_secs(180);

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join(WAVE_FILE))
}

fn read_config(app: &AppHandle) -> Result<WaveConfig, String> {
    match std::fs::read(config_path(app)?) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(WaveConfig::default()),
        Err(error) => Err(error.to_string()),
        // Defaults are safe because the default is "off".
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
    }
}

fn write_config(app: &AppHandle, config: &WaveConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

/// The owner's wave settings.
#[tauri::command]
pub async fn wave_scheduler_config(app: AppHandle) -> Result<WaveConfig, String> {
    tokio::task::spawn_blocking(move || read_config(&app))
        .await
        .map_err(|error| error.to_string())?
}

/// Replace the wave settings.
#[tauri::command]
pub async fn set_wave_scheduler(config: WaveConfig, app: AppHandle) -> Result<WaveConfig, String> {
    tokio::task::spawn_blocking(move || {
        write_config(&app, &config)?;
        Ok(config)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Every agent the scheduler can act on, with its current running state.
fn wave_agents(app: &AppHandle) -> Result<Vec<WaveAgent>, String> {
    let live: std::collections::BTreeSet<(String, String)> = app
        .state::<crate::app_state::AppState>()
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?
        .keys()
        .map(|key| (key.pubkey.clone(), key.relay_url.clone()))
        .collect();

    Ok(crate::managed_agents::load_managed_agents(app)?
        .into_iter()
        .filter(|record| !record.pubkey.is_empty())
        .map(|record| WaveAgent {
            running: live.contains(&(record.pubkey.clone(), record.relay_url.clone())),
            pubkey: record.pubkey,
            relay_url: record.relay_url,
            name: record.name,
            last_turn_at: None,
        })
        .collect())
}

/// One scheduler pass.
pub async fn tick(app: &AppHandle) -> Result<WavePlan, String> {
    let (config, mut agents) = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            Ok::<_, String>((read_config(&app)?, wave_agents(&app)?))
        })
        .await
        .map_err(|error| error.to_string())??
    };

    let plan_out = WavePlan::default();
    if !config.enabled || agents.is_empty() {
        return Ok(plan_out);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    for agent in &mut agents {
        agent.last_turn_at = crate::session_recycler::last_turn_at(app, &agent.pubkey, now).await;
    }

    let plan_out = plan(&config, &agents, now, &|name| {
        crate::managed_agents::cost_profile::is_deep_role(name)
    });

    // Stops first: a place must be free before the agent taking it starts, or
    // the wave momentarily exceeds its own cap.
    let by_key: std::collections::BTreeMap<&str, &WaveAgent> = agents
        .iter()
        .map(|agent| (agent.pubkey.as_str(), agent))
        .collect();

    for pubkey in &plan_out.stop {
        if let Some(agent) = by_key.get(pubkey.as_str()) {
            let _ = crate::managed_agents::stop_managed_agent_runtime(
                agent.pubkey.clone(),
                agent.relay_url.clone(),
                app.clone(),
            );
        }
    }
    for pubkey in &plan_out.start {
        if let Some(agent) = by_key.get(pubkey.as_str()) {
            let _ = crate::managed_agents::start_managed_agent_runtime(
                agent.pubkey.clone(),
                agent.relay_url.clone(),
                app.clone(),
            );
        }
    }
    Ok(plan_out)
}

/// Start the scheduler's background loop for the life of the app.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        loop {
            interval.tick().await;
            match tick(&app).await {
                Ok(plan) if !plan.stop.is_empty() || !plan.start.is_empty() => {
                    eprintln!(
                        "wave-scheduler: stopped {}, started {}, held {} busy",
                        plan.stop.len(),
                        plan.start.len(),
                        plan.held_busy
                    );
                }
                Ok(_) => {}
                Err(error) => eprintln!("wave-scheduler: skipped a pass: {error}"),
            }
        }
    });
}

#[cfg(test)]
#[path = "wave_scheduler_tests.rs"]
mod tests;
