//! Shared native team authority; the desktop and bounded agent tools use the same atomic policy.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// Project roles the owner can assign. A role is a label for responsibility; the
/// capability list is the only thing any tool or gate enforces.
pub const PROJECT_ROLES: &[&str] = &[
    "coordinator",
    "manager",
    "writer",
    "tester",
    "reviewer",
    "member",
];

/// Grantable project capabilities. Publishing, deploying, purchasing and destructive
/// operations are deliberately absent: they are never grantable and always require a
/// per-action owner decision through the approval queue.
pub const PROJECT_CAPABILITIES: &[&str] = &[
    "read",
    "edit",
    "run_checks",
    "manage_team",
    "assign_tasks",
    "review",
    "create_private_channels",
    "commit_merge",
];

/// Upper bound on owner-approved check commands per project.
pub const MAX_APPROVED_CHECKS: usize = 64;

/// One existing agent's project-local grant; never a global identity mutation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectMember {
    /// Existing managed identity public key.
    pub pubkey: String,
    /// One of [`PROJECT_ROLES`].
    pub role: String,
    /// Explicit bounded capabilities from [`PROJECT_CAPABILITIES`]; privileged
    /// side effects are not included.
    pub capabilities: Vec<String>,
}

/// Durable project authority snapshot, fenced by both team and binding revision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectTeam {
    /// Monotonic grant revision, including revocation and pause.
    pub generation: i64,
    /// Binding revision these grants authorize.
    pub binding_generation: i64,
    /// Existing coordinator, absent for an unconfigured team.
    pub coordinator_pubkey: Option<String>,
    /// Project-local members and effective capability grants.
    pub members: Vec<ProjectMember>,
    /// Paused teams authorize no new operations.
    pub paused: bool,
    /// Exact command lines the owner approved for `run_checks` holders. Older
    /// snapshots without this field deserialize as an empty list, which grants
    /// no command.
    #[serde(default)]
    pub approved_checks: Vec<String>,
}

impl Default for ProjectTeam {
    fn default() -> Self {
        Self {
            generation: 0,
            binding_generation: 0,
            coordinator_pubkey: None,
            members: vec![],
            paused: true,
            approved_checks: vec![],
        }
    }
}

/// Validate one owner-approved check command line: bounded, single line, printable.
pub fn validate_check_command(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() || trimmed != command {
        return Err("approved check commands must be trimmed and non-empty".into());
    }
    if command.len() > 512 {
        return Err("approved check command exceeds 512 bytes".into());
    }
    if command.chars().any(|ch| ch.is_control()) {
        return Err("approved check commands must be a single printable line".into());
    }
    Ok(())
}

/// Read one scoped native authority snapshot.
pub fn load(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
) -> Result<ProjectTeam, String> {
    let raw: Option<String> = conn.query_row("SELECT snapshot FROM channel_project_teams WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3",
        params![owner, relay, channel.to_string()], |row| row.get(0)).optional().map_err(|error| error.to_string())?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|error| format!("invalid project team: {error}"))
    })
    .transpose()
    .map(Option::unwrap_or_default)
}

/// Validate bounded roles and capabilities against existing identities.
pub fn validate(team: &ProjectTeam, available: &HashSet<String>) -> Result<(), String> {
    if team.members.len() > 64 {
        return Err("project team exceeds 64 members".into());
    }
    if team.approved_checks.len() > MAX_APPROVED_CHECKS {
        return Err("too many approved check commands".into());
    }
    let mut checks = HashSet::new();
    for command in &team.approved_checks {
        validate_check_command(command)?;
        if !checks.insert(command) {
            return Err("duplicate approved check command".into());
        }
    }
    let mut seen = HashSet::new();
    let mut coordinators = Vec::new();
    for member in &team.members {
        nostr::PublicKey::from_hex(&member.pubkey).map_err(|_| "invalid agent public key")?;
        if !available.contains(&member.pubkey) || !seen.insert(&member.pubkey) {
            return Err("team must contain distinct existing managed agents only".into());
        }
        if !PROJECT_ROLES.contains(&member.role.as_str()) {
            return Err("unsupported project role".into());
        }
        if member.role == "coordinator" {
            coordinators.push(&member.pubkey);
        }
        let mut capabilities = HashSet::new();
        for capability in &member.capabilities {
            if !PROJECT_CAPABILITIES.contains(&capability.as_str())
                || !capabilities.insert(capability)
            {
                return Err("unsupported or duplicate capability; privileged actions require separate owner authorization".into());
            }
        }
    }
    match &team.coordinator_pubkey {
        Some(coordinator) if coordinators == vec![coordinator] => Ok(()),
        None if coordinators.is_empty() => Ok(()),
        _ => Err("select exactly one existing coordinator from this team".into()),
    }
}

/// Reject paused, stale, unconfigured, or insufficient authority.
pub fn authorize(
    team: &ProjectTeam,
    binding_generation: i64,
    generation: i64,
    actor: &str,
    capability: &str,
) -> Result<(), String> {
    if team.paused
        || team.coordinator_pubkey.is_none()
        || team.generation != generation
        || team.binding_generation != binding_generation
    {
        return Err("project authority is paused or stale; reverify this session".into());
    }
    if !team.members.iter().any(|member| {
        member.pubkey == actor && member.capabilities.iter().any(|grant| grant == capability)
    }) {
        return Err("agent does not have this project capability".into());
    }
    Ok(())
}

/// Trusted caller context, never an actor identity supplied in model tool arguments.
pub struct TeamUpdate<'a> {
    /// Current desktop owner.
    pub owner: &'a str,
    /// Canonical relay scope.
    pub relay: &'a str,
    /// Channel being changed.
    pub channel: Uuid,
    /// Authenticated owner or existing runtime identity.
    pub actor: &'a str,
    /// Team revision captured by the caller.
    pub expected_generation: i64,
    /// Binding revision captured by the caller.
    pub expected_binding_generation: i64,
    /// Existing owner-attested public identities.
    pub available: &'a HashSet<String>,
}

/// Validate and persist one authority revision inside the caller's immediate transaction.
pub fn persist(
    transaction: &rusqlite::Transaction<'_>,
    update: TeamUpdate<'_>,
    mut next: ProjectTeam,
    binding_generation: i64,
) -> Result<ProjectTeam, String> {
    let TeamUpdate {
        owner,
        relay,
        channel,
        actor,
        expected_generation,
        expected_binding_generation,
        available,
    } = update;
    if binding_generation != expected_binding_generation {
        return Err("project binding changed; reload the team before saving grants".into());
    }
    let previous = load(transaction, owner, relay, channel)?;
    if previous.generation != expected_generation {
        return Err("project team changed; reload before saving".into());
    }
    validate(&next, available)?;
    if actor != owner {
        authorize(
            &previous,
            binding_generation,
            expected_generation,
            actor,
            "manage_team",
        )?;
        let manager = previous
            .members
            .iter()
            .find(|member| member.pubkey == actor)
            .ok_or("coordinator missing")?;
        if previous.coordinator_pubkey.as_deref() != Some(actor)
            || next.coordinator_pubkey != previous.coordinator_pubkey
            || next.paused != previous.paused
            || next.approved_checks != previous.approved_checks
            || next.members.iter().find(|member| member.pubkey == actor) != Some(manager)
        {
            return Err("only the owner may change coordinator authority, pause, or approved checks; self-escalation is forbidden".into());
        }
        for member in &next.members {
            if previous
                .members
                .iter()
                .find(|prior| prior.pubkey == member.pubkey)
                != Some(member)
                && member
                    .capabilities
                    .iter()
                    .any(|capability| !manager.capabilities.contains(capability))
            {
                return Err("delegation exceeds the coordinator's capability scope".into());
            }
        }
    }
    next.generation = expected_generation
        .checked_add(1)
        .ok_or("team revision exhausted")?;
    if next.generation > 10000 {
        return Err("project authority history limit reached; preserve the audit history".into());
    }
    next.binding_generation = binding_generation;
    let raw = serde_json::to_string(&next).map_err(|error| error.to_string())?;
    transaction.execute("INSERT INTO channel_project_teams(identity_pubkey, relay_url, channel_id, snapshot) VALUES(?1,?2,?3,?4)
        ON CONFLICT(identity_pubkey, relay_url, channel_id) DO UPDATE SET snapshot=excluded.snapshot", params![owner, relay, channel.to_string(), raw]).map_err(|error| error.to_string())?;
    transaction.execute("INSERT INTO channel_project_team_revisions(identity_pubkey, relay_url, channel_id, generation, actor, snapshot, changed_at) VALUES(?1,?2,?3,?4,?5,?6,unixepoch())",
        params![owner, relay, channel.to_string(), next.generation, actor, raw]).map_err(|error| error.to_string())?;
    Ok(next)
}
