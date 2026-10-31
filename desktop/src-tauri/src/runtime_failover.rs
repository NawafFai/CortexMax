//! Move the team to a runtime that still has headroom, and move it home again.
//!
//! When Claude's window is exhausted every Claude model is blocked at once, so
//! the "fall back to a cheaper model" idea does nothing: the cheaper model is
//! behind the same wall. The only fallback that keeps work moving is a
//! different *provider*. Codex has its own separate weekly limit, so a team
//! that cannot spend another Claude token can often keep going on Codex.
//!
//! That is what this does. When a runtime is exhausted and another is healthy,
//! every agent on the exhausted one is rewritten to the healthy one at the
//! closest tier available there, and restarted. When the original recovers,
//! the same agents are moved back to exactly the runtime and model they had.
//!
//! Honesty about "same strength": the destination model is the strongest one
//! this workspace has configured for that runtime, not a measured equivalent.
//! Codex has a single configured model, so a deep role and an execution role
//! both land on it. The move is recorded per agent, so what an agent is
//! running and what it was running are both answerable rather than guessed.
//!
//! What it will not do:
//!
//! * **Move an agent with an explicit harness pin.** `agent_command_override`
//!   wins over everything at spawn, so moving such an agent would rewrite a
//!   field spawn ignores and report a change that never happened. The owner
//!   pinned it deliberately; it is reported as skipped.
//! * **Move on a stale reading.** A window that has already reset still reads
//!   full until its runtime next reports, and migrating a whole team over a
//!   limit that no longer exists would be worse than the limit.
//! * **Move somewhere with no headroom.** The destination must be comfortably
//!   below its own threshold, or a failover just moves the outage.

use serde::{Deserialize, Serialize};

use crate::provider_limits::LimitGroup;

/// Owner settings. Off until switched on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailoverConfig {
    pub enabled: bool,
    /// A runtime at or above this percent is out of room.
    pub exhausted_at_percent: f64,
    /// A runtime must be below this to be worth moving to.
    pub healthy_below_percent: f64,
}

/// Where one record was before it was moved, so it can be put back exactly.
///
/// The store keeps definitions and instances in one file, and both carry a
/// runtime. A linked instance resolves its model from the definition, while
/// the harness command is resolved from the instance's own runtime first — so
/// a migration that wrote only one half would either not take effect or be
/// silently re-mirrored back on the next snapshot apply. One entry is recorded
/// per record, identified by whichever key that half has.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Displaced {
    /// Instance pubkey; empty for a definition.
    pub pubkey: String,
    /// Definition slug; empty for an instance.
    pub slug: String,
    /// Empty for a definition, which has no runtime to restart.
    pub relay_url: String,
    pub name: String,
    /// The runtime it belongs on.
    pub home_runtime: String,
    /// The model it had there, restored verbatim on the way back.
    pub home_model: Option<String>,
}

/// The runtimes a failover would move between.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub from: String,
    pub to: String,
}

/// A runtime's state for routing purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Health {
    Exhausted,
    Healthy,
    /// Neither clearly out of room nor clearly safe to move onto.
    Marginal,
}

/// Grade one runtime from the windows it reports.
///
/// A runtime that reports nothing is `Marginal`, never `Healthy`: moving a
/// team onto a provider whose limits cannot be read is a guess, and moving it
/// off one whose limits cannot be read would be a guess too.
fn health(group: &LimitGroup, config: &FailoverConfig) -> Health {
    let live: Vec<f64> = group
        .windows
        .iter()
        // A window that already reset says nothing about now.
        .filter(|window| !window.stale)
        .map(|window| window.used_percent)
        .collect();
    if live.is_empty() {
        return Health::Marginal;
    }
    let fullest = live.iter().copied().fold(f64::MIN, f64::max);
    if fullest >= config.exhausted_at_percent {
        return Health::Exhausted;
    }
    if fullest < config.healthy_below_percent {
        return Health::Healthy;
    }
    Health::Marginal
}

/// Runtimes agents can actually be moved between.
///
/// OpenCode is deliberately excluded as a destination: its models are free but
/// materially weaker, so an automatic move there would quietly downgrade the
/// team's output while reporting a success.
const MOVABLE: &[&str] = &["claude", "codex"];

/// The route to take, if any.
///
/// Returns the emptiest healthy destination, so a failover buys as much room
/// as the machine has to offer.
pub fn plan_route(config: &FailoverConfig, groups: &[LimitGroup]) -> Option<Route> {
    if !config.enabled {
        return None;
    }

    let graded: Vec<(&LimitGroup, Health)> = groups
        .iter()
        .filter(|group| MOVABLE.contains(&group.runtime.as_str()))
        .map(|group| (group, health(group, config)))
        .collect();

    let from = graded
        .iter()
        .find(|(_, health)| *health == Health::Exhausted)
        .map(|(group, _)| group.runtime.clone())?;

    let to = graded
        .iter()
        .filter(|(group, health)| *health == Health::Healthy && group.runtime != from)
        .min_by(|(left, _), (right, _)| {
            fullest_window(left)
                .partial_cmp(&fullest_window(right))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(group, _)| group.runtime.clone())?;

    Some(Route { from, to })
}

/// The fullest live window a runtime reports, for picking the emptiest one.
fn fullest_window(group: &LimitGroup) -> f64 {
    group
        .windows
        .iter()
        .filter(|window| !window.stale)
        .map(|window| window.used_percent)
        .fold(f64::MIN, f64::max)
}

/// Whether a displaced agent's home runtime has room for it again.
///
/// The bar to come home is the same `healthy_below` used to move: a runtime
/// that has merely dipped under "exhausted" would send the team straight back
/// into the wall.
pub fn can_return(config: &FailoverConfig, groups: &[LimitGroup], home_runtime: &str) -> bool {
    groups
        .iter()
        .find(|group| group.runtime == home_runtime)
        .is_some_and(|group| health(group, config) == Health::Healthy)
}

/// The model to run on `runtime` for a role of this depth.
///
/// This is the strongest model the workspace has configured for that runtime,
/// not a measured equivalent of what the agent was running.
pub fn destination_model(runtime: &str, deep: bool) -> Option<&'static str> {
    crate::managed_agents::cost_profile::CostProfile::Balanced.model_for(runtime, deep)
}

// ---------------------------------------------------------------------------
// Persistence and the pass
// ---------------------------------------------------------------------------

use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

use crate::managed_agents::storage::{
    load_agent_definitions, load_managed_agents, managed_agents_base_dir, save_agent_definitions,
    save_managed_agents,
};
use crate::managed_agents::ManagedAgentRecord;

const FAILOVER_FILE: &str = "runtime-failover.json";
/// Matches the governor's cadence: these decisions read the same cached limit
/// snapshot, and a team migration is not something to do more often than that.
const TICK: std::time::Duration = std::time::Duration::from_secs(120);

/// Settings plus the agents currently away from home.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FailoverRecord {
    pub config: FailoverConfig,
    /// Records moved off their home runtime, and where home is.
    pub displaced: Vec<Displaced>,
    pub last_moved_at: Option<String>,
    /// What the last move did, in plain words.
    pub last_note: Option<String>,
}

impl Default for FailoverConfig {
    fn default() -> Self {
        Self::new_default()
    }
}

impl FailoverConfig {
    fn new_default() -> Self {
        Self {
            enabled: false,
            // Late enough that a normal busy week does not uproot the team.
            exhausted_at_percent: 95.0,
            // Leaves real headroom, so the team is not moved into a second wall
            // an hour later.
            healthy_below_percent: 70.0,
        }
    }
}

fn record_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join(FAILOVER_FILE))
}

fn read_record(app: &AppHandle) -> Result<FailoverRecord, String> {
    match std::fs::read(record_path(app)?) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FailoverRecord::default()),
        Err(error) => Err(error.to_string()),
        // A corrupt record would strand displaced agents, so it is never
        // silently replaced with an empty one — that would lose their way home.
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("failover record is unreadable: {error}")),
    }
}

fn write_record(app: &AppHandle, record: &FailoverRecord) -> Result<(), String> {
    let path = record_path(app)?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?;
    std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

/// Settings and the current displacement.
#[tauri::command]
pub async fn runtime_failover_state(app: AppHandle) -> Result<FailoverRecord, String> {
    tokio::task::spawn_blocking(move || read_record(&app))
        .await
        .map_err(|error| error.to_string())?
}

/// Replace the settings, keeping any displacement so agents can still go home.
#[tauri::command]
pub async fn set_runtime_failover(
    config: FailoverConfig,
    app: AppHandle,
) -> Result<FailoverRecord, String> {
    tokio::task::spawn_blocking(move || {
        let mut record = read_record(&app).unwrap_or_default();
        record.config = config;
        write_record(&app, &record)?;
        Ok(record)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Whether this record may be moved.
///
/// An explicit harness pin wins over `runtime` at spawn, so rewriting one
/// would report a change the harness ignores.
fn is_pinned(record: &ManagedAgentRecord) -> bool {
    record
        .agent_command_override
        .as_deref()
        .map(str::trim)
        .is_some_and(|pin| !pin.is_empty())
}

/// Rewrite one record onto `to`, returning where it came from.
fn move_record(record: &mut ManagedAgentRecord, to: &str, deep: bool) -> Option<Displaced> {
    let home_runtime = record.runtime.clone()?;
    let model = destination_model(to, deep)?;
    let home = Displaced {
        pubkey: record.pubkey.clone(),
        slug: record.slug.clone().unwrap_or_default(),
        relay_url: if record.pubkey.is_empty() {
            String::new()
        } else {
            record.relay_url.clone()
        },
        name: record.name.clone(),
        home_runtime,
        home_model: record.model.clone(),
    };
    record.runtime = Some(to.to_string());
    record.model = Some(model.to_string());
    record.updated_at = crate::util::now_iso();
    Some(home)
}

/// Whether a record is the one this displacement entry describes.
fn matches(record: &ManagedAgentRecord, entry: &Displaced) -> bool {
    if !entry.pubkey.is_empty() {
        return record.pubkey == entry.pubkey;
    }
    !entry.slug.is_empty() && record.slug.as_deref() == Some(entry.slug.as_str())
}

/// Is this agent one of the roles a strong model is kept for?
fn is_deep(name: &str) -> bool {
    crate::managed_agents::cost_profile::is_deep_role(name)
}

/// Move every movable record from `route.from` to `route.to`.
fn migrate(app: &AppHandle, route: &Route) -> Result<(Vec<Displaced>, usize), String> {
    let mut displaced = Vec::new();
    let mut pinned = 0usize;

    let mut definitions = load_agent_definitions(app)?;
    for record in &mut definitions {
        if record.runtime.as_deref() != Some(route.from.as_str()) {
            continue;
        }
        if is_pinned(record) {
            pinned += 1;
            continue;
        }
        if let Some(home) = move_record(record, &route.to, is_deep(&record.name)) {
            displaced.push(home);
        }
    }
    save_agent_definitions(app, &definitions)?;

    let mut instances = load_managed_agents(app)?;
    for record in &mut instances {
        if record.runtime.as_deref() != Some(route.from.as_str()) {
            continue;
        }
        if is_pinned(record) {
            pinned += 1;
            continue;
        }
        if let Some(home) = move_record(record, &route.to, is_deep(&record.name)) {
            displaced.push(home);
        }
    }
    save_managed_agents(app, &instances)?;

    Ok((displaced, pinned))
}

/// Put back every entry whose home runtime has room again.
fn restore(app: &AppHandle, entries: &[Displaced]) -> Result<Vec<Displaced>, String> {
    let mut restored = Vec::new();

    let mut definitions = load_agent_definitions(app)?;
    for record in &mut definitions {
        if let Some(entry) = entries.iter().find(|entry| matches(record, entry)) {
            record.runtime = Some(entry.home_runtime.clone());
            record.model = entry.home_model.clone();
            record.updated_at = crate::util::now_iso();
            restored.push(entry.clone());
        }
    }
    save_agent_definitions(app, &definitions)?;

    let mut instances = load_managed_agents(app)?;
    for record in &mut instances {
        if let Some(entry) = entries.iter().find(|entry| matches(record, entry)) {
            record.runtime = Some(entry.home_runtime.clone());
            record.model = entry.home_model.clone();
            record.updated_at = crate::util::now_iso();
            restored.push(entry.clone());
        }
    }
    save_managed_agents(app, &instances)?;

    Ok(restored)
}

/// Restart the running agents among `entries`, so the new runtime takes effect.
///
/// A rewritten record only changes what the *next* launch uses, so an agent
/// left running would keep burning the runtime it was moved off.
fn restart_running(app: &AppHandle, entries: &[Displaced]) -> usize {
    let live: std::collections::BTreeSet<(String, String)> = app
        .state::<crate::app_state::AppState>()
        .managed_agent_processes
        .lock()
        .map(|processes| {
            processes
                .keys()
                .map(|key| (key.pubkey.clone(), key.relay_url.clone()))
                .collect()
        })
        .unwrap_or_default();

    entries
        .iter()
        .filter(|entry| !entry.pubkey.is_empty())
        .filter(|entry| live.contains(&(entry.pubkey.clone(), entry.relay_url.clone())))
        .filter(|entry| {
            crate::managed_agents::restart_managed_agent_runtime(
                entry.pubkey.clone(),
                entry.relay_url.clone(),
                app.clone(),
            )
            .is_ok()
        })
        .count()
}

use tauri::Manager;

/// One failover pass: go home if home has room, otherwise leave if stuck.
pub async fn tick(app: &AppHandle) -> Result<Option<String>, String> {
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

    // Coming home takes priority: an agent belongs on its own runtime, and
    // leaving it away once the limit cleared would keep the team split.
    if !record.displaced.is_empty() {
        let ready: Vec<Displaced> = record
            .displaced
            .iter()
            .filter(|entry| can_return(&record.config, &groups, &entry.home_runtime))
            .cloned()
            .collect();
        if ready.is_empty() {
            return Ok(None);
        }

        let app_for_restore = app.clone();
        let ready_for_restore = ready.clone();
        let restored =
            tokio::task::spawn_blocking(move || restore(&app_for_restore, &ready_for_restore))
                .await
                .map_err(|error| error.to_string())??;
        let restarted = restart_running(app, &restored);

        let note = format!(
            "moved {} record(s) back to their own runtime; restarted {restarted}",
            restored.len()
        );
        let app_for_write = app.clone();
        let note_for_write = note.clone();
        tokio::task::spawn_blocking(move || {
            let mut latest = read_record(&app_for_write)?;
            latest
                .displaced
                .retain(|entry| !ready.iter().any(|done| done == entry));
            latest.last_moved_at = Some(crate::util::now_iso());
            latest.last_note = Some(note_for_write);
            write_record(&app_for_write, &latest)
        })
        .await
        .map_err(|error| error.to_string())??;

        let _ = app.emit("runtime-failover-moved", &note);
        return Ok(Some(note));
    }

    let Some(route) = plan_route(&record.config, &groups) else {
        return Ok(None);
    };

    let app_for_migrate = app.clone();
    let route_for_migrate = route.clone();
    let (displaced, pinned) =
        tokio::task::spawn_blocking(move || migrate(&app_for_migrate, &route_for_migrate))
            .await
            .map_err(|error| error.to_string())??;
    if displaced.is_empty() {
        return Ok(None);
    }
    let restarted = restart_running(app, &displaced);

    let note = format!(
        "{} is out of room, so {} record(s) moved to {}; restarted {restarted}{}",
        route.from,
        displaced.len(),
        route.to,
        if pinned == 0 {
            String::new()
        } else {
            format!(". {pinned} left alone — pinned to a harness")
        }
    );

    let app_for_write = app.clone();
    let note_for_write = note.clone();
    tokio::task::spawn_blocking(move || {
        let mut latest = read_record(&app_for_write)?;
        latest.displaced.extend(displaced);
        latest.last_moved_at = Some(crate::util::now_iso());
        latest.last_note = Some(note_for_write);
        write_record(&app_for_write, &latest)
    })
    .await
    .map_err(|error| error.to_string())??;

    let _ = app.emit("runtime-failover-moved", &note);
    Ok(Some(note))
}

/// Start the failover's background loop for the life of the app.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        loop {
            interval.tick().await;
            match tick(&app).await {
                Ok(Some(note)) => eprintln!("runtime-failover: {note}"),
                Ok(None) => {}
                Err(error) => eprintln!("runtime-failover: skipped a pass: {error}"),
            }
        }
    });
}

#[cfg(test)]
#[path = "runtime_failover_tests.rs"]
mod tests;
