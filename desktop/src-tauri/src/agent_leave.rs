//! Which agents are out of runway, until when, and who stands in for them.
//!
//! A spent limit does not stop an agent politely: it fails mid-turn with a
//! provider error, retries, and fails again. The owner sees an agent that
//! looks busy and produces nothing — which is exactly how two of this team's
//! managers burned 38 and 26 retries in a row while appearing to work.
//!
//! So the state is named rather than inferred from failures. An agent whose
//! window is spent is *on leave*, with the provider's own reset time as the
//! return date, and a stand-in named while it is away.
//!
//! The leave decision is evidence: it comes from the provider's published
//! window. The stand-in is a designation, because the app knows what an agent
//! is called and what it runs, not what it is for — picking a replacement from
//! a name would be a guess wearing a badge. So a cover is either set by the
//! owner or suggested here with its reasoning visible, and never silently
//! assumed.

use crate::provider_limits::{LimitGroup, LimitWindow};

/// Percent at which an agent's window counts as spent.
///
/// Matches [`crate::model_failover::SPENT_AT`] on purpose: an agent the
/// failover would move is the same agent the roster should show as away. Two
/// thresholds would let the panel disagree with the action.
pub(crate) const SPENT_AT: f64 = 95.0;

/// One agent considered for leave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Member {
    pub pubkey: String,
    pub name: String,
    /// Provider family, e.g. `claude`, `codex`, `kimi`.
    pub runtime: String,
    /// Model id it is set to.
    pub model: String,
    /// A cover the owner designated, if any.
    pub designated_cover: Option<String>,
}

/// Why an agent is away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cause {
    /// The window for this agent's specific model is spent.
    ModelSpent(String),
    /// The provider's account-wide window is spent.
    ProviderSpent,
    /// The provider reports no credit at all, so there is nothing to wait for.
    NoCredit,
}

/// Who is standing in, and on what basis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cover {
    /// The owner named this agent.
    Designated(String),
    /// Nobody was named; this is the best available, with the reason.
    Suggested { name: String, because: String },
    /// Nobody is available, and saying so is the honest answer.
    None { because: String },
}

/// One agent's leave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Leave {
    pub pubkey: String,
    pub name: String,
    pub cause: Cause,
    /// Unix seconds when the window resets, when the provider stated one.
    ///
    /// `None` means the provider gave no reset — a credit balance does not
    /// refill on a clock, so no countdown is shown rather than a made-up one.
    pub returns_at: Option<i64>,
    pub cover: Cover,
}

fn spent(window: &LimitWindow) -> bool {
    !window.stale && window.used_percent >= SPENT_AT
}

/// The account-wide window for a provider, if it published one.
fn account_window(group: &LimitGroup) -> Option<&LimitWindow> {
    group.windows.iter().find(|w| w.model.is_none())
}

/// Why this member is away, if it is.
fn cause_for(member: &Member, groups: &[LimitGroup]) -> Option<(Cause, Option<i64>)> {
    let group = groups.iter().find(|g| g.runtime == member.runtime)?;

    // A provider that reports no credit is not on a timer: there is nothing to
    // count down to, so the reason is stated without a return date.
    if group
        .note
        .as_deref()
        .is_some_and(|note| note.to_ascii_lowercase().contains("no credit"))
    {
        return Some((Cause::NoCredit, None));
    }

    // The agent's own model first: a spent model with an account that still has
    // room is a narrower fact than "the provider is out", and it is the one an
    // owner can act on by switching model.
    for window in &group.windows {
        if let Some(model) = window.model.as_deref() {
            if spent(window) && crate::model_failover::model_matches(model, &member.model) {
                return Some((Cause::ModelSpent(model.to_string()), window.resets_at));
            }
        }
    }

    let account = account_window(group)?;
    if spent(account) {
        return Some((Cause::ProviderSpent, account.resets_at));
    }
    None
}

/// Pick a stand-in from the members who are not themselves away.
///
/// Preference order, and the reason each step exists:
///  1. Whoever the owner designated — a human judgement outranks this one.
///  2. An available agent on a *different* provider, because a peer on the same
///     spent provider is behind the same wall and would be away by morning.
///  3. Any available agent at all.
fn cover_for(member: &Member, available: &[&Member]) -> Cover {
    if let Some(named) = member.designated_cover.as_deref() {
        return Cover::Designated(named.to_string());
    }
    if available.is_empty() {
        return Cover::None {
            because: "every other agent is on leave too".to_string(),
        };
    }
    if let Some(other) = available.iter().find(|m| m.runtime != member.runtime) {
        return Cover::Suggested {
            name: other.name.clone(),
            because: format!("on {}, which still has room", other.runtime),
        };
    }
    Cover::None {
        because: "the only agents still working are on the same spent provider".to_string(),
    }
}

/// Work out who is away and who covers them.
pub(crate) fn roll_call(groups: &[LimitGroup], members: &[Member]) -> Vec<Leave> {
    let away: Vec<(&Member, Cause, Option<i64>)> = members
        .iter()
        .filter_map(|m| cause_for(m, groups).map(|(cause, at)| (m, cause, at)))
        .collect();

    let away_keys: Vec<&str> = away.iter().map(|(m, _, _)| m.pubkey.as_str()).collect();
    let available: Vec<&Member> = members
        .iter()
        .filter(|m| !away_keys.contains(&m.pubkey.as_str()))
        .collect();

    away.into_iter()
        .map(|(member, cause, returns_at)| Leave {
            pubkey: member.pubkey.clone(),
            name: member.name.clone(),
            cause,
            returns_at,
            cover: cover_for(member, &available),
        })
        .collect()
}

/// A duration written the way someone reads a wait.
///
/// Coarse on purpose: "back in 2h" is what an owner acts on, and a ticking
/// "1h 59m 58s" invites watching a clock instead of working. Minutes appear
/// only under an hour, where they are the difference between waiting and
/// going to do something else.
fn humanise(seconds: i64) -> String {
    let minutes = seconds / 60;
    if minutes < 1 {
        return "under a minute".to_string();
    }
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        let rest = minutes % 60;
        return if rest == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {rest}m")
        };
    }
    let days = hours / 24;
    let rest = hours % 24;
    if rest == 0 {
        format!("{days}d")
    } else {
        format!("{days}d {rest}h")
    }
}

/// One line for the roster badge.
pub(crate) fn describe(leave: &Leave, now: i64) -> String {
    let reason = match &leave.cause {
        Cause::ModelSpent(model) => format!("{model} is spent"),
        Cause::ProviderSpent => "its provider's window is spent".to_string(),
        Cause::NoCredit => "its provider has no credit left".to_string(),
    };
    let back = match leave.returns_at {
        // A reset already in the past describes nothing current; the agent is
        // due back rather than away for a negative length of time.
        Some(at) if at > now => format!("back in {}", humanise(at - now)),
        Some(_) => "due back now".to_string(),
        None => "no reset scheduled".to_string(),
    };
    format!("On leave — {reason}, {back}")
}

#[cfg(test)]
#[path = "agent_leave_tests.rs"]
mod tests;

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// One agent's leave, as the roster shows it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaveEntry {
    pub pubkey: String,
    pub name: String,
    /// The badge line, already written for a person to read.
    pub label: String,
    /// Unix seconds of the return, or `null` when there is nothing to count to.
    pub returns_at: Option<i64>,
    /// Who is standing in, or `null` when nobody can.
    pub cover: Option<String>,
    /// Why that cover, so a suggestion never reads as a decision.
    pub cover_note: Option<String>,
}

/// Who on the roster is out of runway right now.
///
/// Reads the providers' own windows, so it reports what the provider says
/// rather than inferring from failures after the fact.
#[tauri::command]
pub async fn agent_roll_call(app: tauri::AppHandle) -> Result<Vec<LeaveEntry>, String> {
    let groups = crate::provider_limits::get_provider_limits(None).await?;
    let members = tokio::task::spawn_blocking({
        let app = app.clone();
        move || {
            crate::managed_agents::load_managed_agents(&app)
                .unwrap_or_default()
                .into_iter()
                .filter(|record| !record.pubkey.is_empty())
                .filter_map(|record| {
                    let model = record.model.clone()?;
                    Some(Member {
                        pubkey: record.pubkey.clone(),
                        name: record.name.clone(),
                        runtime: provider_family(&model).to_string(),
                        model,
                        // Designation lives with the owner, not in this record
                        // yet; until there is a field for it, every cover is a
                        // suggestion and is labelled as one.
                        designated_cover: None,
                    })
                })
                .collect::<Vec<_>>()
        }
    })
    .await
    .map_err(|error| error.to_string())?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);

    Ok(roll_call(&groups, &members)
        .into_iter()
        .map(|leave| {
            let (cover, cover_note) = match &leave.cover {
                Cover::Designated(name) => (Some(name.clone()), None),
                Cover::Suggested { name, because } => (Some(name.clone()), Some(because.clone())),
                Cover::None { because } => (None, Some(because.clone())),
            };
            LeaveEntry {
                label: describe(&leave, now),
                pubkey: leave.pubkey,
                name: leave.name,
                returns_at: leave.returns_at,
                cover,
                cover_note,
            }
        })
        .collect())
}

/// The provider a model id belongs to.
///
/// By id prefix rather than by a configured runtime string: the record's
/// harness field and the limit panel's runtime key are different vocabularies,
/// and matching the wrong one is how a feature ends up silently inert.
fn provider_family(model: &str) -> &'static str {
    if model.starts_with("claude-") {
        "claude"
    } else if model.starts_with("gpt-") {
        "codex"
    } else if model.starts_with("kimi") {
        "kimi"
    } else if model.starts_with("opencode") {
        "opencode"
    } else {
        ""
    }
}
