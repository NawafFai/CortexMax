use super::{
    teams::{self, ProjectMember, TeamUpdate},
    ProjectPolicy,
};
use buzz_core::local_workspace::ProjectSessionScope;
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io::Read};

#[derive(Deserialize)]
struct CandidateRecord {
    pubkey: String,
    name: String,
    auth_tag: Option<String>,
}

#[derive(Serialize)]
struct Candidate {
    pubkey: String,
    name: String,
}

fn candidates(policy: &ProjectPolicy) -> Result<Vec<Candidate>, String> {
    let file = std::fs::File::open(&policy.agent_store)
        .map_err(|_| "existing-agent metadata unavailable; no identities were changed")?;
    if file.metadata().map_err(|error| error.to_string())?.len() > 16 * 1024 * 1024 {
        return Err("existing-agent metadata exceeds limit".into());
    }
    let records: Vec<CandidateRecord> = serde_json::from_reader(file.take(16 * 1024 * 1024 + 1))
        .map_err(|_| "invalid existing-agent metadata; no identities were changed")?;
    if records.len() > 4096 {
        return Err("existing-agent inventory exceeds bounded limit".into());
    }
    Ok(records
        .into_iter()
        .filter_map(|record| {
            let pubkey = nostr::PublicKey::from_hex(&record.pubkey).ok()?;
            let owner =
                buzz_sdk::nip_oa::verify_auth_tag(record.auth_tag.as_deref()?, &pubkey).ok()?;
            (owner.to_hex() == policy.owner_pubkey).then_some(Candidate {
                pubkey: record.pubkey,
                name: record.name,
            })
        })
        .collect())
}

pub(super) fn operate(
    policy: &ProjectPolicy,
    scope: &ProjectSessionScope,
    next: Option<Vec<ProjectMember>>,
) -> Result<String, String> {
    let mut conn = policy.open()?;
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let capability = if next.is_some() {
        "manage_team"
    } else {
        "read"
    };
    let current = policy
        .resolve(&transaction, &scope.channel_id.to_string(), capability)?
        .ok_or("project detached")?;
    if current != *scope {
        return Err("native team session is stale; reverify before delegation".into());
    }
    let mut team = teams::load(
        &transaction,
        &scope.owner_pubkey,
        &scope.relay_url,
        scope.channel_id,
    )?;
    let available = if team.coordinator_pubkey.as_deref() == Some(policy.agent_pubkey.as_str())
        && team.members.iter().any(|member| {
            member.pubkey == policy.agent_pubkey
                && member
                    .capabilities
                    .iter()
                    .any(|capability| capability == "manage_team")
        }) {
        candidates(policy)?
    } else {
        vec![]
    };
    let changed = next.is_some();
    if let Some(members) = next {
        super::edits::require_reconciled(&transaction, &scope.relay_url)?;
        let keys: HashSet<String> = available
            .iter()
            .map(|candidate| candidate.pubkey.clone())
            .collect();
        team.members = members;
        team = teams::persist(
            &transaction,
            TeamUpdate {
                owner: &scope.owner_pubkey,
                relay: &scope.relay_url,
                channel: scope.channel_id,
                actor: &policy.agent_pubkey,
                expected_generation: scope.team_generation,
                expected_binding_generation: scope.binding_generation,
                available: &keys,
            },
            team,
            scope.binding_generation,
        )?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"team":team,"available_agents":available,"session_invalidated":changed,"message":if changed {"Project grants changed. This session is stale; a new authorized session is required. Global identities were not changed."}else{"Project-local grants only; not a runtime readiness claim."}}).to_string())
}
