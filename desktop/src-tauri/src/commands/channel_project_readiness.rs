//! Structured project readiness: one check per fact, a derived state, and the time
//! each fact was established. Nothing here starts a runtime or a model; it reads
//! the device-local authority store, the runtime catalog, the process table and
//! the stored inspection evidence, and it says exactly which of those it could
//! not establish.

use crate::{
    app_state::AppState,
    archive::{channel_workspaces, project_readiness, project_tasks, project_teams::ProjectTeam},
    managed_agents::{storage::ProjectAgentMetadata, ProjectEnforcementSummary, ProjectLaunchKind},
};
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

/// One readiness fact.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ReadinessCheck {
    /// Stable identifier, e.g. `folder`, `git`, `member:<pubkey>`.
    pub id: String,
    /// `ok`, `warn`, `fail`, or `unknown`.
    pub status: String,
    /// What was established.
    pub message: String,
    /// What the owner can do about it, when it is not `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
}

/// Git facts read from the repository files (no process is run).
#[derive(Clone, Debug, Serialize, Default, PartialEq, Eq)]
pub struct GitFacts {
    /// Current branch, if the root is a repository with a symbolic HEAD.
    pub branch: Option<String>,
    /// Revision, if resolvable from loose or packed refs.
    pub revision: Option<String>,
    /// Uncommitted changes, known only from the last isolated inspection.
    pub dirty: Option<bool>,
    /// Unix seconds of the inspection that established `dirty`.
    pub verified_at: Option<i64>,
}

/// Structured readiness for one channel project.
#[derive(Clone, Debug, Serialize)]
pub struct ProjectReadiness {
    /// The readiness ladder, cumulative: `conversation_only`, `selected`,
    /// `attached`, `runtime_configured`, `access_verified`,
    /// `execution_authorized`, `task_running`.
    pub state: String,
    /// Unix seconds this computation ran.
    pub checked_at: u64,
    /// Every fact, in display order.
    pub checks: Vec<ReadinessCheck>,
    /// Messages of every `fail` check, for callers that only need blockers.
    pub blockers: Vec<String>,
    /// Git facts.
    pub git: GitFacts,
    /// Unix seconds of the last isolated inspection that still matches the
    /// current binding and team generations; `None` when access is unverified.
    pub last_inspection_at: Option<i64>,
    /// Owner decisions waiting in the approval queue.
    pub pending_approvals: i64,
    /// Tasks currently in the `running` state.
    pub running_tasks: i64,
}

fn check(
    id: &str,
    status: &str,
    message: impl Into<String>,
    recovery: Option<&str>,
) -> ReadinessCheck {
    ReadinessCheck {
        id: id.to_string(),
        status: status.to_string(),
        message: message.into(),
        recovery: recovery.map(str::to_string),
    }
}

/// Read branch and revision from `.git` files without running Git.
pub(crate) fn git_head(root: &Path) -> Option<(Option<String>, Option<String>)> {
    let git_dir = root.join(".git");
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref: ") {
        let branch = reference
            .strip_prefix("refs/heads/")
            .map(str::to_string)
            .or_else(|| Some(reference.to_string()));
        let revision = std::fs::read_to_string(git_dir.join(reference))
            .ok()
            .map(|raw| raw.trim().to_string())
            .filter(|raw| !raw.is_empty())
            .or_else(|| {
                let packed = std::fs::read_to_string(git_dir.join("packed-refs")).ok()?;
                packed.lines().find_map(|line| {
                    let (hash, name) = line.split_once(' ')?;
                    (name.trim() == reference).then(|| hash.trim().to_string())
                })
            });
        Some((branch, revision))
    } else if head.len() >= 7 {
        Some((None, Some(head.to_string())))
    } else {
        None
    }
}

/// Compute readiness. `team` is the saved team; `agents` the owner-attested
/// existing agents the desktop knows about.
pub(crate) fn compute(
    app: &AppHandle,
    conn: &rusqlite::Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    team: &ProjectTeam,
    agents: &[ProjectAgentMetadata],
) -> Result<ProjectReadiness, String> {
    let mut checks = Vec::new();
    let mut git = GitFacts::default();
    let binding = channel_workspaces::list(conn, owner, relay)?
        .into_iter()
        .find(|binding| binding.channel_id == channel);
    let root = binding.as_ref().and_then(|binding| binding.root.clone());
    let binding_generation = binding
        .as_ref()
        .map(|binding| binding.generation)
        .unwrap_or(0);

    // Ladder rungs, each proven by the checks that precede it.
    let mut attached = false;
    let mut runtime_configured = true;
    let mut access_verified = false;

    match &root {
        None => checks.push(check(
            "binding",
            "fail",
            "No local project is attached to this channel.",
            Some("Choose an existing folder or create a new project from the Workspace tab."),
        )),
        Some(root) => {
            checks.push(check(
                "binding",
                "ok",
                format!("Project attached (binding revision {binding_generation})."),
                None,
            ));
            match channel_workspaces::validate_root(root) {
                Ok(canonical) => {
                    attached = true;
                    checks.push(check(
                        "folder",
                        "ok",
                        "Project folder exists and is readable by this desktop.",
                        None,
                    ));
                    match git_head(&canonical) {
                        Some((branch, revision)) => {
                            git.branch = branch.clone();
                            git.revision = revision.clone();
                            checks.push(check(
                                "git",
                                "ok",
                                format!(
                                    "Git branch {} at {}.",
                                    branch.as_deref().unwrap_or("(detached)"),
                                    revision.as_deref().map(|r| &r[..r.len().min(12)]).unwrap_or("unknown revision")
                                ),
                                None,
                            ));
                        }
                        None => checks.push(check(
                            "git",
                            "warn",
                            "The folder is not a Git repository (or HEAD is unreadable); uncommitted-change tracking is unavailable.",
                            Some("Initialize a repository if you want revision and dirty-state evidence."),
                        )),
                    }
                }
                Err(error) => {
                    checks.push(check(
                        "folder",
                        "fail",
                        format!("Project folder unavailable: {error}"),
                        Some("Restore the folder at its original path, or detach and attach the moved folder."),
                    ));
                    checks.push(check(
                        "git",
                        "unknown",
                        "Git state cannot be read while the folder is unavailable.",
                        None,
                    ));
                }
            }
        }
    }

    // Team facts.
    if team.binding_generation != binding_generation && root.is_some() {
        runtime_configured = false;
        checks.push(check(
            "team_binding",
            "fail",
            "The project binding changed after these grants were saved; they no longer authorize anything.",
            Some("Open Team and permissions, review the grants, and save them again."),
        ));
    }
    match &team.coordinator_pubkey {
        Some(_) => checks.push(check("coordinator", "ok", "A coordinator is chosen.", None)),
        None => {
            runtime_configured = false;
            checks.push(check(
                "coordinator",
                "fail",
                "No coordinator is chosen.",
                Some("Pick an existing agent as coordinator in Team and permissions."),
            ));
        }
    }
    if team.members.is_empty() {
        runtime_configured = false;
        checks.push(check(
            "team",
            "fail",
            "The project team is empty.",
            Some("Add at least one existing agent to the team."),
        ));
    }
    if team.paused {
        checks.push(check(
            "paused",
            "warn",
            "Project dispatch is paused; no new task is assigned or started.",
            Some("Resume the project when you are ready."),
        ));
    }

    // Per-member runtime facts.
    let processes = app
        .state::<AppState>()
        .managed_agent_processes
        .lock()
        .map(|processes| {
            processes
                .iter()
                .filter(|(key, _)| key.relay_url == relay)
                .map(|(key, pair)| {
                    (
                        key.pubkey.clone(),
                        (pair.process.project_launch, pair.lifecycle.clone()),
                    )
                })
                .collect::<std::collections::HashMap<_, _>>()
        })
        .map_err(|error| error.to_string())?;
    for member in &team.members {
        let id = format!("member:{}", member.pubkey);
        let Some(agent) = agents.iter().find(|agent| agent.pubkey == member.pubkey) else {
            runtime_configured = false;
            checks.push(check(
                &id,
                "fail",
                format!(
                    "Agent {} is missing or its owner attestation no longer verifies.",
                    member.pubkey
                ),
                Some("Release the member, or restore the agent on this device."),
            ));
            continue;
        };
        let name = agent.name.as_str();
        let runtime_id = agent.runtime.as_deref();
        let enforcement =
            crate::managed_agents::project_enforcement::summary_for_runtime_id(runtime_id);
        let expected_launch = match &enforcement {
            ProjectEnforcementSummary::NativeSandbox => Some(ProjectLaunchKind::NativeBounded),
            ProjectEnforcementSummary::Gate { profile } => {
                match profile.enforceable_for(&member.capabilities) {
                    Ok(()) => Some(ProjectLaunchKind::HarnessGated),
                    Err(reason) => {
                        runtime_configured = false;
                        checks.push(check(&format!("{id}:enforcement"), "fail", format!("{name}: {reason}"), Some("Adjust the member's grants or choose another runtime for this member.")));
                        None
                    }
                }
            }
            ProjectEnforcementSummary::Unverified { reason } => {
                runtime_configured = false;
                checks.push(check(
                    &format!("{id}:enforcement"),
                    "fail",
                    format!(
                        "{name} runs on `{}` which has no verified project enforcement: {reason}",
                        runtime_id.unwrap_or("unknown")
                    ),
                    Some("Release this member or switch it to Claude Code, Codex, or Buzz Agent."),
                ));
                None
            }
        };
        if let Some(expected) = expected_launch {
            let boundary = match &enforcement {
                ProjectEnforcementSummary::NativeSandbox => "native sandboxed tools".to_string(),
                ProjectEnforcementSummary::Gate { profile } => format!(
                    "harness gate (writes {:?}, reads {:?}, commands {:?}, network {})",
                    profile.write_confinement,
                    profile.read_confinement,
                    profile.command_confinement,
                    if profile.network_blocked {
                        "blocked"
                    } else {
                        "open"
                    }
                ),
                ProjectEnforcementSummary::Unverified { .. } => "unverified".to_string(),
            };
            checks.push(check(
                &format!("{id}:enforcement"),
                "ok",
                format!("{name}: {boundary}."),
                None,
            ));
            if !agent.local {
                runtime_configured = false;
                checks.push(check(&format!("{id}:runtime"), "fail", format!("{name} is not managed on this device; its runtime cannot carry this project's authority."), Some("Release the member, or manage the agent locally.")));
            } else {
                match processes.get(&member.pubkey) {
                    Some((launch, lifecycle)) => {
                        if *launch == expected {
                            checks.push(check(&format!("{id}:runtime"), "ok", format!("{name} runtime is {lifecycle:?} with project authority ({expected:?})."), None));
                        } else {
                            runtime_configured = false;
                            checks.push(check(&format!("{id}:runtime"), "fail", format!("{name} runtime was launched as {launch:?}; the current team requires {expected:?}."), Some("Restart this agent so it launches with the current project authority.")));
                        }
                    }
                    None => checks.push(check(
                        &format!("{id}:runtime"),
                        "warn",
                        format!("{name} runtime is not running."),
                        Some("Start the agent; it will launch with this project's authority."),
                    )),
                }
            }
            match runtime_id.map(crate::managed_agents::cached_auth_status) {
                Some(crate::managed_agents::AuthStatus::LoggedIn) => checks.push(check(&format!("{id}:auth"), "ok", format!("{name} runtime CLI reports a valid login."), None)),
                Some(crate::managed_agents::AuthStatus::LoggedOut) => {
                    runtime_configured = false;
                    checks.push(check(&format!("{id}:auth"), "fail", format!("{name} runtime CLI is not logged in."), Some("Run the runtime's login command, then refresh runtime discovery.")));
                }
                Some(crate::managed_agents::AuthStatus::ConfigInvalid { diagnostic }) => {
                    runtime_configured = false;
                    checks.push(check(&format!("{id}:auth"), "fail", format!("{name} runtime CLI configuration is invalid: {diagnostic}"), Some("Fix the runtime's configuration file, then refresh runtime discovery.")));
                }
                Some(crate::managed_agents::AuthStatus::NotApplicable) | None => checks.push(check(&format!("{id}:auth"), "ok", format!("{name} runtime has no separate login step."), None)),
                Some(crate::managed_agents::AuthStatus::Unknown) => checks.push(check(&format!("{id}:auth"), "warn", format!("{name} runtime login status has not been probed in this session."), Some("Refresh runtime discovery (Agents → runtimes) to run the provider's own login check."))),
            }
        }
    }

    // Queues and unresolved side effects.
    let pending_approvals = if root.is_some() {
        buzz_agent_pkg::project_workspace::approvals::count_pending(conn, owner, relay, channel)?
    } else {
        0
    };
    if pending_approvals > 0 {
        checks.push(check(
            "approvals",
            "warn",
            format!("{pending_approvals} refused action(s) await your decision."),
            Some("Open the Approvals tab to approve or deny each request."),
        ));
    }
    if let Err(error) = buzz_agent_pkg::project_workspace::edits::require_reconciled_for_channel(
        conn,
        relay,
        &channel.to_string(),
    ) {
        runtime_configured = false;
        checks.push(check(
            "edits",
            "fail",
            error,
            Some("Open Edit recovery and reconcile the pending edit receipt."),
        ));
    }
    if let Err(error) = crate::archive::workspace_provider_attempts::require_released(conn, relay) {
        runtime_configured = false;
        checks.push(check(
            "provider_attempts",
            "fail",
            error,
            Some("Confirm the remote writer has stopped in the Workspace tab."),
        ));
    }
    if let Err(error) =
        crate::archive::project_channel_creations::require_settled(conn, owner, relay)
    {
        runtime_configured = false;
        checks.push(check(
            "channel_creation",
            "fail",
            error,
            Some("Resume or abandon the pending channel creation from the create-channel form."),
        ));
    }

    // Handoff from a previous session: a captured checkpoint whose previous
    // writer has not been confirmed released keeps execution unauthorized.
    if root.is_some() {
        match crate::archive::project_handoffs::load(conn, owner, relay, channel)? {
            Some(handoff) if handoff.previous_writer_released_at.is_none() => {
                runtime_configured = false;
                checks.push(check(
                    "handoff",
                    "fail",
                    format!(
                        "A handoff checkpoint ({}: {}) is recorded but the previous writer's release has not been confirmed.",
                        handoff.source, handoff.label
                    ),
                    Some("Review the handoff in the Review tab and confirm that the previous writer has stopped."),
                ));
            }
            Some(handoff) => checks.push(check(
                "handoff",
                "ok",
                format!(
                    "Handoff checkpoint ({}: {}) reviewed; previous writer confirmed released.",
                    handoff.source, handoff.label
                ),
                None,
            )),
            None => checks.push(check(
                "handoff",
                "warn",
                "No handoff checkpoint is recorded for this project.",
                Some("If another session worked here, inspect the handoff file or import its summary in the Review tab."),
            )),
        }
    }

    // Inspection evidence.
    let mut last_inspection_at = None;
    if root.is_some() {
        match project_readiness::load(conn, owner, relay, channel)? {
            Some(inspection) if inspection.binding_generation == binding_generation && inspection.team_generation == team.generation => {
                access_verified = true;
                last_inspection_at = Some(inspection.checked_at);
                git.dirty = inspection.dirty;
                git.verified_at = Some(inspection.checked_at);
                if git.branch.is_none() {
                    git.branch = inspection.branch.clone();
                }
                if git.revision.is_none() {
                    git.revision = inspection.revision.clone();
                }
                checks.push(check("inspection", "ok", format!("Actual working directory verified by isolated inspection ({}).", inspection.actual_cwd), None));
                if inspection.dirty == Some(true) {
                    checks.push(check("dirty", "warn", "Uncommitted changes are present in the project; they are preserved, never reset.", Some("Commit or stash them yourself if you want a clean baseline; agents never do this for you.")));
                }
            }
            Some(_) => checks.push(check("inspection", "warn", "The last isolated inspection predates the current binding or team revision.", Some("Run the isolated local inspection again."))),
            None => checks.push(check("inspection", "warn", "Actual working directory, branch and dirty state have not been verified by an isolated inspection yet.", Some("Run the isolated local inspection in Team and permissions."))),
        }
    }

    let running_tasks = if root.is_some() {
        project_tasks::count_in_state(conn, owner, relay, channel, "running")?
    } else {
        0
    };
    let blockers: Vec<String> = checks
        .iter()
        .filter(|check| check.status == "fail")
        .map(|check| check.message.clone())
        .collect();
    let state = if root.is_none() {
        "conversation_only"
    } else if !attached {
        "selected"
    } else if !runtime_configured || !blockers.is_empty() {
        "attached"
    } else if !access_verified {
        "runtime_configured"
    } else if team.paused {
        "access_verified"
    } else if running_tasks > 0 {
        "task_running"
    } else {
        "execution_authorized"
    };
    Ok(ProjectReadiness {
        state: state.to_string(),
        checked_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs(),
        checks,
        blockers,
        git,
        last_inspection_at,
        pending_approvals,
        running_tasks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_head_reads_symbolic_and_detached_heads_without_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(git_head(root).is_none());
        std::fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/feature\n").unwrap();
        assert_eq!(git_head(root), Some((Some("feature".into()), None)));
        std::fs::write(root.join(".git/refs/heads/feature"), "abc123\n").unwrap();
        assert_eq!(
            git_head(root),
            Some((Some("feature".into()), Some("abc123".into())))
        );
        std::fs::remove_file(root.join(".git/refs/heads/feature")).unwrap();
        std::fs::write(
            root.join(".git/packed-refs"),
            "# pack\ndeadbeef refs/heads/feature\n",
        )
        .unwrap();
        assert_eq!(
            git_head(root),
            Some((Some("feature".into()), Some("deadbeef".into())))
        );
        std::fs::write(root.join(".git/HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(
            git_head(root),
            Some((None, Some("0123456789abcdef".into())))
        );
    }
}
