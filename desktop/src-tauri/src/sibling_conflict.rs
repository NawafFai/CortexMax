//! Warn when another Buzz-family app owns the same agent identities.
//!
//! Cortex Max is a fork, so a machine that ran upstream Buzz first will have
//! both apps holding the *same* agent keypairs. Nothing prevents both from
//! running at once, and when they do, two processes sign as the same agent and
//! connect to the same relay under one identity. The relay has no way to tell
//! them apart: turns interleave, one app's reply answers the other's prompt,
//! and the duplicated identity burns two subscriptions' worth of tokens on one
//! conversation.
//!
//! When a team was imported from Buzz the overlap is total — every agent key is shared — so the
//! warning is not hypothetical.
//!
//! Two things this module will not do:
//!
//! * **It never writes.** The sibling's store is read to compare public keys
//!   and nothing else. Cortex Max does not adopt, migrate or modify another
//!   app's agents.
//! * **It never touches private keys.** [`SiblingAgent`] deliberately declares
//!   only the two public fields it needs, so the sibling's `private_key_nsec`
//!   is not deserialized, not held in memory, and cannot leak into an error
//!   message or a log line.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How much trouble the overlap is actually causing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// No shared identities, or no sibling installed.
    None,
    /// Identities are shared, but the sibling is not running them.
    Warning,
    /// The sibling is running agents that Cortex Max also owns, right now.
    Critical,
}

/// Exactly the two public fields needed to compare stores.
///
/// Declaring the struct this narrowly is the safety mechanism, not an
/// optimization: serde ignores every other field, so the sibling's private key
/// never enters this process.
#[derive(Debug, Deserialize)]
struct SiblingAgent {
    #[serde(default)]
    pubkey: String,
    #[serde(default)]
    name: String,
}

/// What another Buzz-family install shares with this one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictReport {
    /// Bundle identifier of the sibling app.
    pub bundle_id: String,
    /// Agent identities both apps hold.
    pub shared_agents: usize,
    /// Of those, the ones the sibling currently has running.
    pub running_shared_agents: usize,
    pub severity: Severity,
    /// A few shared agent names, so the owner recognizes their own team.
    pub sample_names: Vec<String>,
}

/// Buzz-family app-data directory names to look for beside our own.
///
/// Upstream Buzz first; a demo variant, where present, holds
/// its own store, so it can conflict in exactly the same way.
const SIBLING_DIRS: &[&str] = &["xyz.block.buzz.app", "Buzz"];

/// Grade an overlap.
///
/// Sharing identities is always worth saying, because it means the two apps
/// can never safely run together — but it is only actively breaking things
/// while the sibling is running them.
pub fn assess(shared_agents: usize, running_shared_agents: usize) -> Severity {
    if shared_agents == 0 {
        return Severity::None;
    }
    if running_shared_agents > 0 {
        return Severity::Critical;
    }
    Severity::Warning
}

/// Read a store's public agent identities: `(pubkey, name)`, key-less
/// definition rows excluded.
fn read_public_identities(store: &Path) -> Vec<(String, String)> {
    let Ok(bytes) = std::fs::read(store) else {
        return Vec::new();
    };
    let Ok(agents) = serde_json::from_slice::<Vec<SiblingAgent>>(&bytes) else {
        return Vec::new();
    };
    agents
        .into_iter()
        .filter(|agent| !agent.pubkey.is_empty())
        .map(|agent| (agent.pubkey, agent.name))
        .collect()
}

/// The agent pubkeys a store currently has running, by checking that each
/// recorded pid is still alive.
///
/// A stale pid file left by a crash must not be reported as a running agent,
/// which would cry wolf every time the sibling had ever been used.
fn running_pubkeys(agents_dir: &Path, is_alive: &dyn Fn(u32) -> bool) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(agents_dir.join("agent-pids")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let pubkey = name.to_str()?.strip_suffix(".pid")?.to_string();
            let pid: u32 = std::fs::read_to_string(entry.path())
                .ok()?
                .trim()
                .parse()
                .ok()?;
            is_alive(pid).then_some(pubkey)
        })
        .collect()
}

/// Whether a pid is a live process.
#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    // Signal 0 performs the permission and existence checks without delivering
    // anything, which is the standard way to ask "is this pid alive".
    // SAFETY: `kill` with signal 0 has no effect beyond returning a status.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> bool {
    // Without a cheap liveness check, report the overlap as a warning rather
    // than claiming agents are running that may not be.
    false
}

/// Compare one sibling store against our own identities.
fn inspect_sibling(
    bundle_id: &str,
    sibling_agents_dir: &Path,
    ours: &std::collections::BTreeSet<String>,
    is_alive: &dyn Fn(u32) -> bool,
) -> Option<ConflictReport> {
    let theirs = read_public_identities(&sibling_agents_dir.join("managed-agents.json"));
    if theirs.is_empty() {
        return None;
    }

    let shared: Vec<(String, String)> = theirs
        .into_iter()
        .filter(|(pubkey, _)| ours.contains(pubkey))
        .collect();
    if shared.is_empty() {
        return None;
    }

    let running: std::collections::BTreeSet<String> = running_pubkeys(sibling_agents_dir, is_alive)
        .into_iter()
        .collect();
    let running_shared = shared
        .iter()
        .filter(|(pubkey, _)| running.contains(pubkey))
        .count();

    Some(ConflictReport {
        bundle_id: bundle_id.to_string(),
        shared_agents: shared.len(),
        running_shared_agents: running_shared,
        severity: assess(shared.len(), running_shared),
        sample_names: shared
            .iter()
            .filter(|(_, name)| !name.is_empty())
            .take(4)
            .map(|(_, name)| name.clone())
            .collect(),
    })
}

/// Every Buzz-family install that shares identities with this one.
#[tauri::command]
pub async fn sibling_conflicts(app: tauri::AppHandle) -> Result<Vec<ConflictReport>, String> {
    tokio::task::spawn_blocking(move || {
        let ours: std::collections::BTreeSet<String> =
            crate::managed_agents::load_managed_agents(&app)?
                .into_iter()
                .map(|record| record.pubkey)
                .filter(|pubkey| !pubkey.is_empty())
                .collect();
        if ours.is_empty() {
            return Ok(Vec::new());
        }

        let Some(support) = app_support_dir() else {
            return Ok(Vec::new());
        };
        Ok(SIBLING_DIRS
            .iter()
            .filter_map(|dir| {
                inspect_sibling(dir, &support.join(dir).join("agents"), &ours, &pid_is_alive)
            })
            .collect())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// The platform directory holding per-app support data.
fn app_support_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|home| home.join("Library/Application Support"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::data_dir()
    }
}

#[cfg(test)]
#[path = "sibling_conflict_tests.rs"]
mod tests;
