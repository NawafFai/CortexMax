//! Cost profiles: move a whole team between model tiers in one action.
//!
//! Spend is dominated by which model each agent runs on, and the only way to
//! change it was to edit every agent by hand — sixty-odd edits at the exact
//! moment the owner is trying to stop a runaway run. A profile applies one
//! policy to the whole team: a strong model for the few roles whose judgement
//! the rest of the work depends on, a cheap one for everyone else, and a
//! concurrency cap so a single agent cannot hold several sessions open at once.
//!
//! The split is by role, not by preference. Coordination and review decide what
//! the rest of the team does, so a weak model there costs more than it saves;
//! execution roles write against requirements that already exist, which the
//! cheap tier handles. [`DEEP_ROLES`] is that list, and it matches the roles the
//! live team actually uses.
//!
//! Only model IDs this workspace has actually run are used here. An ID the
//! harness cannot match is not a spawn failure — it warns and silently falls
//! back to the agent's default model — so an invented "premium" ID would look
//! applied while changing nothing. Codex and OpenCode each have exactly one
//! configured model, so profiles change only their concurrency, and
//! [`CostProfileOutcome::capped_only`] reports that rather than implying a tier
//! change that did not happen.
//!
//! The store keeps definitions and instances in one file, split by whether the
//! record has a pubkey. Both halves are written: a linked instance resolves its
//! model from the *definition* (see `effective_config::resolve_effective_config`),
//! while `parallelism` is read off the *instance* at spawn — so writing only one
//! half would leave the profile half-applied. Nothing is started or stopped, so
//! a profile switch is safe to press mid-run; each agent picks its new model up
//! on its next launch.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::storage::{
    load_agent_definitions, load_managed_agents, save_agent_definitions, save_managed_agents,
};
use super::types::ManagedAgentRecord;

/// Roles whose judgement the rest of the team depends on. Everyone else is an
/// execution role for profile purposes.
const DEEP_ROLES: &[&str] = &[
    "CORTEX",
    "Pilot Project Manager",
    "Judge Technical Reviewer",
    "Scope Product and Requirements Analyst",
    "Prism Software Architect",
    "Orion Research Scientist",
    "Axel Process Engineer",
];

/// Models this workspace has run. Anything outside this set would fall back to
/// the agent default without telling the owner.
const CLAUDE_OPUS: &str = "claude-opus-5-5";
const CLAUDE_SONNET: &str = "claude-sonnet-5";
const CLAUDE_HAIKU: &str = "claude-haiku-4-5-20251001";
const CODEX_MODEL: &str = "gpt-5.6-terra";
const OPENCODE_MODEL: &str = "opencode/nemotron-3.5-lightning-free";
/// Kimi Code's own catalogue, read from its config after `kimi login`. The
/// split is the provider's own: "highspeed" is named as the fast tier, and
/// `kimi-for-coding` is the flagship it picks as the default for coding.
/// Its K3 models are deliberately not tiered here — nothing in their names or
/// metadata ranks them against these two, and guessing an order would put a
/// role on a model chosen by assumption.
const KIMI_DEEP: &str = "kimi-code/kimi-for-coding";
const KIMI_FAST: &str = "kimi-code/kimi-for-coding-highspeed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CostProfile {
    /// Cheapest tier everywhere, one session per agent.
    Economy,
    /// Strong models for coordination and review, cheap for execution.
    Balanced,
    /// Strongest tier available; what an unconstrained team costs.
    Max,
}

impl CostProfile {
    /// The model this profile wants for `runtime`, or `None` when the runtime
    /// has no tier to choose between.
    pub(crate) fn model_for(self, runtime: &str, deep: bool) -> Option<&'static str> {
        match runtime {
            "claude" => Some(match (self, deep) {
                (Self::Economy, _) => CLAUDE_HAIKU,
                (Self::Balanced, true) => CLAUDE_SONNET,
                (Self::Balanced, false) => CLAUDE_HAIKU,
                (Self::Max, true) => CLAUDE_OPUS,
                (Self::Max, false) => CLAUDE_SONNET,
            }),
            // One configured model each: the profile still caps concurrency,
            // but claiming a tier change here would be false.
            "codex" => Some(CODEX_MODEL),
            "opencode" => Some(OPENCODE_MODEL),
            "kimi" => Some(match (self, deep) {
                (Self::Economy, _) => KIMI_FAST,
                (Self::Balanced, true) => KIMI_DEEP,
                (Self::Balanced, false) => KIMI_FAST,
                (Self::Max, _) => KIMI_DEEP,
            }),
            _ => None,
        }
    }

    /// Concurrent sessions allowed per agent: (deep roles, execution roles).
    fn parallelism(self, deep: bool) -> u32 {
        match (self, deep) {
            (Self::Economy, _) => 1,
            (Self::Balanced, true) => 2,
            (Self::Balanced, false) => 1,
            (Self::Max, true) => 4,
            (Self::Max, false) => 2,
        }
    }

    /// Whether this runtime's model is fixed regardless of profile.
    fn is_single_model_runtime(runtime: &str) -> bool {
        matches!(runtime, "codex" | "opencode")
    }
}

/// Whether this role is one the team's direction depends on, and so keeps a
/// strong model. Shared with runtime failover, which picks a destination model
/// by the same split.
pub(crate) fn is_deep_role(name: &str) -> bool {
    DEEP_ROLES.contains(&name)
}

/// What applying a profile changed, so the UI reports the actual effect rather
/// than a blanket success.
///
/// Every count is a number of *agents*, not of records. An agent normally owns
/// two records — a definition and an instance — so counting records would tell
/// a thirty-agent owner that sixty agents changed.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostProfileOutcome {
    /// Agents whose model moved to a different tier.
    pub retiered: usize,
    /// Agents on a single-model runtime: concurrency changed, model could not.
    pub capped_only: usize,
    /// Agents already matching this profile.
    pub unchanged: usize,
    /// Agents with no recognized runtime, left untouched.
    pub skipped: usize,
    /// A few names that moved tier, for the confirmation message.
    pub examples: Vec<String>,
}

/// What a profile did to one record. Ordered by how much it is worth
/// reporting, so an agent whose two records disagree is summarized by the more
/// significant half rather than by whichever was visited last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Applied {
    Skipped,
    Unchanged,
    Capped,
    Retiered,
}

/// Apply `profile` to one record, reporting what it did.
fn apply_to(record: &mut ManagedAgentRecord, profile: CostProfile) -> Applied {
    let Some(runtime) = record.runtime.clone() else {
        return Applied::Skipped;
    };
    let deep = DEEP_ROLES.contains(&record.name.as_str());
    let model = profile.model_for(&runtime, deep);
    // A runtime this workspace knows but has no tier for still gets its
    // concurrency set — that is the part of a profile which applies to every
    // runtime alike, and skipping it would leave one team uncapped.
    if model.is_none() && !CostProfile::is_single_model_runtime(&runtime) {
        return Applied::Skipped;
    }
    let target_parallelism = profile.parallelism(deep);

    let model_changed = model.is_some_and(|model| record.model.as_deref() != Some(model));
    // Definitions carry their own concurrency field; instances carry the one
    // spawn reads. A record is only ever one of the two.
    let is_definition = record.pubkey.is_empty();
    let parallelism_changed = if is_definition {
        record.definition_parallelism != Some(target_parallelism)
    } else {
        record.parallelism != target_parallelism
    };

    if !model_changed && !parallelism_changed {
        return Applied::Unchanged;
    }

    if let Some(model) = model {
        record.model = Some(model.to_string());
    }
    if is_definition {
        record.definition_parallelism = Some(target_parallelism);
    } else {
        record.parallelism = target_parallelism;
    }
    record.updated_at = crate::util::now_iso();

    if CostProfile::is_single_model_runtime(&runtime) {
        return Applied::Capped;
    }
    Applied::Retiered
}

/// Fold one record's result into the per-agent tally, keeping the most
/// significant outcome seen for that agent.
fn note(tally: &mut BTreeMap<String, Applied>, name: &str, applied: Applied) {
    let slot = tally.entry(name.to_string()).or_insert(applied);
    *slot = (*slot).max(applied);
}

/// Turn the per-agent tally into the owner-facing counts.
fn summarize(tally: BTreeMap<String, Applied>) -> CostProfileOutcome {
    let mut outcome = CostProfileOutcome::default();
    for (name, applied) in tally {
        match applied {
            Applied::Retiered => {
                outcome.retiered += 1;
                if outcome.examples.len() < 6 {
                    outcome.examples.push(name);
                }
            }
            Applied::Capped => outcome.capped_only += 1,
            Applied::Unchanged => outcome.unchanged += 1,
            Applied::Skipped => outcome.skipped += 1,
        }
    }
    outcome
}

/// Apply a cost profile to every managed agent.
#[tauri::command]
pub async fn apply_cost_profile(
    profile: CostProfile,
    app: AppHandle,
) -> Result<CostProfileOutcome, String> {
    tokio::task::spawn_blocking(move || {
        let mut tally: BTreeMap<String, Applied> = BTreeMap::new();

        // Definitions first: a linked instance resolves its model from here, so
        // if the second save fails the authoritative half is already correct.
        let mut definitions = load_agent_definitions(&app)?;
        for record in &mut definitions {
            let applied = apply_to(record, profile);
            note(&mut tally, &record.name, applied);
        }
        save_agent_definitions(&app, &definitions)?;

        let mut instances = load_managed_agents(&app)?;
        for record in &mut instances {
            let applied = apply_to(record, profile);
            note(&mut tally, &record.name, applied);
        }
        save_managed_agents(&app, &instances)?;

        Ok(summarize(tally))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// The profile the store currently matches, or `None` when it matches none —
/// so the UI can highlight the active profile instead of guessing.
#[tauri::command]
pub async fn current_cost_profile(app: AppHandle) -> Result<Option<CostProfile>, String> {
    tokio::task::spawn_blocking(move || {
        let definitions = load_agent_definitions(&app)?;
        Ok([
            CostProfile::Economy,
            CostProfile::Balanced,
            CostProfile::Max,
        ]
        .into_iter()
        .find(|profile| matches_profile(&definitions, *profile)))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Whether every tier-carrying record already sits on `profile`. Runtimes with
/// a single model are ignored: they match every profile and would otherwise
/// make an empty team look like whichever profile is checked first.
fn matches_profile(records: &[ManagedAgentRecord], profile: CostProfile) -> bool {
    let mut saw_one = false;
    for record in records {
        let Some(runtime) = record.runtime.as_deref() else {
            continue;
        };
        if CostProfile::is_single_model_runtime(runtime) {
            continue;
        }
        let deep = DEEP_ROLES.contains(&record.name.as_str());
        let Some(model) = profile.model_for(runtime, deep) else {
            continue;
        };
        if record.model.as_deref() != Some(model) {
            return false;
        }
        saw_one = true;
    }
    saw_one
}

#[cfg(test)]
#[path = "cost_profile_tests.rs"]
mod tests;
