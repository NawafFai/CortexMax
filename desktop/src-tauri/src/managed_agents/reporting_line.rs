//! Who reports to whom, and what that makes "inside the house".
//!
//! The reporting line used to live only in prompt text, which meant the code
//! could not act on it. Copying a manager on a cross-department request was
//! therefore a rule each agent had to remember, and an agent that forgot sent
//! a second `@mention` instead — waking the manager it was trying merely to
//! inform, which is the exact cost the copy exists to avoid.
//!
//! Held as data, the same rule becomes something the system applies. This
//! module answers the one question the sending side needs: for this agent,
//! which other agents count as its own house?
//!
//! A message inside the house is ordinary work and gets copied to nobody.
//! Crossing out of it spends someone else's capacity, and that is when the
//! sender's manager is copied — by the tool, not by the agent's good memory.

use std::collections::{HashMap, HashSet};

use super::types::ManagedAgentRecord;

/// One agent's reporting context, ready to hand to a spawning process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReportingContext {
    /// The agent this one reports to, if any. `None` means the owner.
    pub manager: Option<String>,
    /// Everyone a message may reach without copying the manager: the manager
    /// itself, peers under that same manager, and this agent's own reports at
    /// any depth.
    pub same_house: Vec<String>,
}

fn normalize(pubkey: &str) -> String {
    pubkey.trim().to_ascii_lowercase()
}

/// Build the reporting context for `agent` from the whole roster.
///
/// The three groups are each here for their own reason:
///
/// - **The manager.** Copying your manager on a message *to* your manager is
///   addressing and informing the same person twice.
/// - **Peers under the same manager.** Two engineers in one department
///   working together is the department doing its job; a copy on every such
///   message would bury the manager in its own team's ordinary traffic.
/// - **Reports, at any depth.** Directing your own people is the authority
///   you already hold, so there is nothing to inform anyone about.
///
/// Everyone else is another department's, and reaching them spends capacity
/// this agent does not own.
pub(crate) fn reporting_context(
    agent_pubkey: &str,
    roster: &[ManagedAgentRecord],
) -> ReportingContext {
    let me = normalize(agent_pubkey);
    let manager_of: HashMap<String, Option<String>> = roster
        .iter()
        .map(|record| {
            (
                normalize(&record.pubkey),
                record
                    .manager_pubkey
                    .as_deref()
                    .map(normalize)
                    // An agent recorded as its own manager is a data error, not
                    // a root: treating it as a root keeps the walk finite and
                    // loses nothing a cycle could have told us.
                    .filter(|boss| boss != &me || boss != &normalize(&record.pubkey)),
            )
        })
        .collect();

    let manager = manager_of.get(&me).cloned().flatten();

    let mut house: HashSet<String> = HashSet::new();
    if let Some(ref boss) = manager {
        house.insert(boss.clone());
        // Peers: everyone else reporting to the same manager.
        for record in roster {
            let pubkey = normalize(&record.pubkey);
            if pubkey != me && manager_of.get(&pubkey).cloned().flatten().as_ref() == Some(boss) {
                house.insert(pubkey);
            }
        }
    }

    // Reports, walked breadth-first. The visited set is what keeps a cycle in
    // the stored data — which a hand-edited file can always contain — from
    // spinning here rather than at the point that wrote it.
    let mut frontier = vec![me.clone()];
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(me.clone());
    while let Some(current) = frontier.pop() {
        for record in roster {
            let pubkey = normalize(&record.pubkey);
            if manager_of.get(&pubkey).cloned().flatten().as_deref() != Some(current.as_str()) {
                continue;
            }
            if visited.insert(pubkey.clone()) {
                house.insert(pubkey.clone());
                frontier.push(pubkey);
            }
        }
    }

    house.remove(&me);
    let mut same_house: Vec<String> = house.into_iter().collect();
    // Sorted so the value handed to a spawning process is stable: an unstable
    // one would look like a config change and restart agents for nothing.
    same_house.sort();
    ReportingContext {
        manager,
        same_house,
    }
}

#[cfg(test)]
#[path = "reporting_line_tests.rs"]
mod tests;
