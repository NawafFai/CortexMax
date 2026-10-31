//! Move a team off one spent model before moving it off its provider.
//!
//! Claude publishes a separate weekly window for each premium model beside an
//! account-wide one, so the two can disagree sharply: a panel can read
//! `Week · Fable` at 100% while `Week · all models` sits at 72%. One model is
//! spent; the subscription is not. The cross-provider failover cannot help
//! there — it treats a provider as a single wall — so the team would be
//! migrated to another provider while most of the one it is on stands unused.
//!
//! This decides the cheaper move first: keep the agents where they are and
//! change which model they ask for. Only when the account-wide window is also
//! spent is the provider genuinely out of room, and that is the case the
//! existing [`crate::runtime_failover`] already handles.
//!
//! Everything here is a pure decision over the limits and the roster. It
//! plans; it does not write. That is deliberate — a wrong plan that was never
//! applied is a bug report, and a wrong plan that rewrote sixty agent records
//! is an incident.

use crate::provider_limits::{LimitGroup, LimitWindow};

/// Percent at which a model's own window counts as spent.
///
/// Below 100 on purpose: a model that reports 99% has, in practice, nothing
/// useful left, and waiting for a round 100 spends the last turns discovering
/// what the number already said.
pub(crate) const SPENT_AT: f64 = 95.0;

/// Headroom the account-wide window must still have for a move to be worth
/// making. A switch that lands on a provider which is itself nearly spent
/// buys minutes, and costs a rewrite of every affected agent to get them.
pub(crate) const ACCOUNT_HEADROOM_AT: f64 = 85.0;

/// One agent considered for a model move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub pubkey: String,
    pub relay_url: String,
    pub name: String,
    /// The harness it runs on, e.g. `claude`.
    pub runtime: String,
    /// The model it is currently set to.
    pub model: String,
}

/// An agent to move, and where to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Move {
    pub pubkey: String,
    pub relay_url: String,
    pub name: String,
    pub runtime: String,
    pub from_model: String,
    pub to_model: String,
}

/// Why no move was planned, in the owner's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Blocked {
    /// No model-scoped window is spent.
    NothingSpent,
    /// A model is spent, but so is the account as a whole — this is a
    /// provider-level problem, not a model-level one.
    AccountAlsoSpent,
    /// A model is spent and no unspent sibling model is known for its runtime.
    NoHealthySibling,
    /// A model is spent but no agent is currently set to it.
    NoAgentsOnSpentModel,
}

/// The plan: what to move, or why nothing is being moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    Move(Vec<Move>),
    Blocked(Blocked),
}

/// Whether a provider's window names the model an agent is set to.
///
/// These are two vocabularies and the difference is not cosmetic. Claude's CLI
/// writes a display name — `week (Fable)`, `week (Opus)` — while an agent
/// record holds a model id like `claude-fable-5-1`. Comparing the two for
/// equality never matches, so a failover written that way sits silently inert
/// in exactly the situation it exists for. Matching the display name as a
/// case-insensitive fragment of the id is what actually connects the limit to
/// the roster.
pub(crate) fn model_matches(window_model: &str, agent_model: &str) -> bool {
    let needle = window_model.trim().to_ascii_lowercase();
    !needle.is_empty() && agent_model.to_ascii_lowercase().contains(&needle)
}

/// Models a runtime can be switched to, dearest first.
///
/// Ordering matters: the point of the move is to keep working at the closest
/// capability still available, so the first unspent sibling is taken rather
/// than the cheapest. Dropping straight to the cheapest tier is what the cost
/// profile is for, and it is the owner's decision, not this module's.
pub(crate) fn siblings(runtime: &str) -> &'static [&'static str] {
    match runtime {
        // Opus 5.5 replaced Opus 5 in the Claude adapter's own model list
        // (0.81): `opus[1m]` now names 5.5, and Opus 5 is no longer offered.
        "claude" => &[
            "claude-fable-5-1",
            "claude-opus-5-5",
            "claude-sonnet-5",
            "claude-haiku-4-5-20251001",
        ],
        _ => &[],
    }
}

/// Whether this window says its scope is spent.
///
/// A stale window is never spent: it describes a window that has already
/// reset, and acting on it would migrate a team away from a limit that is no
/// longer there.
fn spent(window: &LimitWindow, at: f64) -> bool {
    !window.stale && window.used_percent >= at
}

/// The account-wide window for a group, if it published one.
fn account_window(group: &LimitGroup) -> Option<&LimitWindow> {
    group
        .windows
        .iter()
        .find(|window| window.model.is_none() && window.window_minutes.unwrap_or(0) > 24 * 60)
}

/// Plan a within-provider model move, or explain why there is none.
///
/// The order of the checks is the argument: a model being spent is only
/// actionable while the account still has room, because otherwise the move
/// lands on the same wall from a different direction.
pub(crate) fn plan(groups: &[LimitGroup], roster: &[Candidate]) -> Plan {
    let mut saw_spent_model = false;
    let mut saw_account_spent = false;

    for group in groups {
        for window in &group.windows {
            let Some(model) = window.model.as_deref() else {
                continue;
            };
            if !spent(window, SPENT_AT) {
                continue;
            }
            saw_spent_model = true;

            // An account-wide window that is itself nearly spent makes this a
            // provider problem. Absent one, there is no evidence of headroom,
            // and moving on no evidence is guessing — so it is also refused.
            let has_headroom =
                account_window(group).is_some_and(|account| !spent(account, ACCOUNT_HEADROOM_AT));
            if !has_headroom {
                saw_account_spent = true;
                continue;
            }

            let spent_models: Vec<&str> = group
                .windows
                .iter()
                .filter(|other| spent(other, SPENT_AT))
                .filter_map(|other| other.model.as_deref())
                .collect();

            // The destination must be a sibling this runtime actually offers
            // and one that is not itself spent. A model with no window of its
            // own is covered by the account-wide window, which was just shown
            // to have room.
            let destination = siblings(&group.runtime)
                .iter()
                .find(|candidate| {
                    !model_matches(model, candidate)
                        && !spent_models
                            .iter()
                            .any(|spent| model_matches(spent, candidate))
                })
                .copied();
            let Some(destination) = destination else {
                continue;
            };

            let moves: Vec<Move> = roster
                .iter()
                .filter(|agent| {
                    agent.runtime == group.runtime && model_matches(model, &agent.model)
                })
                .map(|agent| Move {
                    pubkey: agent.pubkey.clone(),
                    relay_url: agent.relay_url.clone(),
                    name: agent.name.clone(),
                    runtime: agent.runtime.clone(),
                    from_model: model.to_string(),
                    to_model: destination.to_string(),
                })
                .collect();

            if !moves.is_empty() {
                return Plan::Move(moves);
            }
        }
    }

    // The order of these is the order the owner would ask them in: is anything
    // spent at all, is it the account rather than a model, and only then the
    // narrower reasons.
    Plan::Blocked(if !saw_spent_model {
        Blocked::NothingSpent
    } else if saw_account_spent {
        Blocked::AccountAlsoSpent
    } else if roster.is_empty() {
        Blocked::NoAgentsOnSpentModel
    } else {
        // A spent model with account headroom and no move left means either
        // no sibling was available or no agent was on it. Distinguish them so
        // the message does not send the owner looking in the wrong place.
        let on_spent = groups.iter().any(|group| {
            group.windows.iter().any(|window| {
                window.model.as_deref().is_some_and(|model| {
                    spent(window, SPENT_AT)
                        && roster.iter().any(|agent| {
                            agent.runtime == group.runtime && model_matches(model, &agent.model)
                        })
                })
            })
        });
        if on_spent {
            Blocked::NoHealthySibling
        } else {
            Blocked::NoAgentsOnSpentModel
        }
    })
}

/// One line saying what the plan is, for the owner.
pub(crate) fn summarize(plan: &Plan) -> String {
    match plan {
        Plan::Move(moves) => {
            let Some(first) = moves.first() else {
                return "Nothing to move.".to_string();
            };
            format!(
                "{} agent{} on {} would move to {} — {} is spent while the account still has room.",
                moves.len(),
                if moves.len() == 1 { "" } else { "s" },
                first.from_model,
                first.to_model,
                first.from_model,
            )
        }
        Plan::Blocked(Blocked::NothingSpent) => {
            "No model has spent its own weekly window.".to_string()
        }
        Plan::Blocked(Blocked::AccountAlsoSpent) => {
            "A model is spent, but so is the account-wide window — switching model within this provider would not help.".to_string()
        }
        Plan::Blocked(Blocked::NoHealthySibling) => {
            "A model is spent and every model this runtime can switch to is spent too.".to_string()
        }
        Plan::Blocked(Blocked::NoAgentsOnSpentModel) => {
            "A model is spent, but no agent is currently set to it.".to_string()
        }
    }
}

/// The plan plus its one-line explanation, for the UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFailoverReport {
    /// Empty when nothing is being moved.
    pub moves: Vec<ReportedMove>,
    pub summary: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportedMove {
    pub name: String,
    pub from_model: String,
    pub to_model: String,
}

/// Every agent the store knows a model for, from both halves of it.
///
/// Definitions and instances are both read because a linked instance resolves
/// its model from its definition — a roster built from instances alone would
/// miss the record that actually decides what runs.
fn roster(app: &tauri::AppHandle) -> Vec<Candidate> {
    let mut out = Vec::new();
    for record in crate::managed_agents::load_agent_definitions(app)
        .unwrap_or_default()
        .into_iter()
        .chain(crate::managed_agents::load_managed_agents(app).unwrap_or_default())
    {
        let Some(model) = record.model.clone() else {
            continue;
        };
        out.push(Candidate {
            pubkey: record.pubkey.clone(),
            relay_url: record.relay_url.clone(),
            name: record.name.clone(),
            // A model name identifies its provider — `claude-opus-5` exists
            // only on Claude — so the runtime is taken from the model's own
            // family rather than from a harness id that varies per record.
            runtime: runtime_of(&model).to_string(),
            model,
        });
    }
    out
}

/// The provider a model belongs to, or an empty family when it is unknown.
fn runtime_of(model: &str) -> &'static str {
    // By id prefix rather than by the switch ladder: an agent on a Claude
    // model this build does not list as a destination is still on Claude, and
    // calling it runtime-less would hide it from the check that decides
    // whether anything is affected at all.
    if model.starts_with("claude-") {
        "claude"
    } else {
        ""
    }
}

/// What a within-provider model move would do right now. Read-only.
#[tauri::command]
pub async fn model_failover_plan(app: tauri::AppHandle) -> Result<ModelFailoverReport, String> {
    let groups = crate::provider_limits::get_provider_limits(None).await?;
    let candidates = tokio::task::spawn_blocking({
        let app = app.clone();
        move || roster(&app)
    })
    .await
    .map_err(|error| error.to_string())?;

    let decided = plan(&groups, &candidates);
    let summary = summarize(&decided);
    Ok(ModelFailoverReport {
        moves: match &decided {
            Plan::Move(moves) => moves
                .iter()
                .map(|entry| ReportedMove {
                    name: entry.name.clone(),
                    from_model: entry.from_model.clone(),
                    to_model: entry.to_model.clone(),
                })
                .collect(),
            Plan::Blocked(_) => Vec::new(),
        },
        summary,
    })
}

/// Carry out the move the planner describes.
///
/// It re-plans rather than trusting anything passed in: the limits may have
/// reset between the panel being drawn and the button being pressed, and
/// rewriting sixty agent records on a stale plan is the expensive mistake here.
#[tauri::command]
pub async fn apply_model_failover(app: tauri::AppHandle) -> Result<ModelFailoverReport, String> {
    let groups = crate::provider_limits::get_provider_limits(Some(true)).await?;
    tokio::task::spawn_blocking(move || {
        let candidates = roster(&app);
        let decided = plan(&groups, &candidates);
        let summary = summarize(&decided);
        let Plan::Move(moves) = &decided else {
            return Ok(ModelFailoverReport {
                moves: Vec::new(),
                summary,
            });
        };
        let Some(first) = moves.first() else {
            return Ok(ModelFailoverReport {
                moves: Vec::new(),
                summary,
            });
        };
        let (from, to) = (first.from_model.clone(), first.to_model.clone());

        // Definitions first, for the same reason the cost profile does it:
        // a linked instance resolves its model from here, so if the second
        // save fails the authoritative half is already correct.
        let mut definitions = crate::managed_agents::load_agent_definitions(&app)?;
        let mut changed = Vec::new();
        for record in &mut definitions {
            if record.model.as_deref() == Some(from.as_str()) {
                record.model = Some(to.clone());
                changed.push(record.name.clone());
            }
        }
        crate::managed_agents::save_agent_definitions(&app, &definitions)?;

        let mut instances = crate::managed_agents::load_managed_agents(&app)?;
        for record in &mut instances {
            if record.model.as_deref() == Some(from.as_str()) {
                record.model = Some(to.clone());
                changed.push(record.name.clone());
            }
        }
        crate::managed_agents::save_managed_agents(&app, &instances)?;

        // One agent owns a definition and an instance, so the same name can be
        // rewritten twice; report agents, not records.
        changed.sort();
        changed.dedup();
        Ok(ModelFailoverReport {
            moves: changed
                .into_iter()
                .map(|name| ReportedMove {
                    name,
                    from_model: from.clone(),
                    to_model: to.clone(),
                })
                .collect(),
            summary,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
#[path = "model_failover_tests.rs"]
mod tests;
