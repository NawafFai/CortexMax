use anyhow::{ensure, Context, Result};
use buzz_core::local_workspace::{ChannelWorkspaceSnapshot, WORKSPACE_SNAPSHOT_ENV};

pub(crate) fn resolve_owner(
    agent: &nostr::PublicKey,
    auth_tag: Option<&str>,
    fallback: Option<&str>,
) -> Option<String> {
    if let Some(auth_tag) = auth_tag.filter(|tag| !tag.is_empty()) {
        match buzz_sdk::nip_oa::verify_auth_tag(auth_tag, agent) {
            Ok(owner) => return Some(owner.to_hex()),
            Err(error) => {
                tracing::warn!("BUZZ_AUTH_TAG verification failed: {error} — falling back")
            }
        }
    }
    fallback.map(str::to_owned)
}

/// Resolve the process working directory without pretending it enforces access.
pub(crate) fn current_working_directory() -> Result<String> {
    let cwd = std::env::current_dir().context("failed to resolve current working directory")?;
    ensure!(
        cwd.is_absolute(),
        "current working directory is not absolute: {}",
        cwd.display()
    );
    Ok(cwd.to_string_lossy().into_owned())
}

/// Whether a catalog harness id matches the configured agent command. The catalog
/// declares enforcement facts per harness id; the harness must never accept a
/// profile written for a different adapter.
pub(crate) fn gate_profile_matches_command(harness: &str, command: &str) -> bool {
    let identity = crate::config::normalize_agent_command_identity(command);
    match harness {
        "claude" => matches!(
            identity.as_str(),
            "claude-agent-acp" | "claude-code-acp" | "claude-code" | "claudecode"
        ),
        "codex" => matches!(identity.as_str(), "codex" | "codex-acp"),
        _ => false,
    }
}

pub(crate) fn validate_launch(
    relay: &str,
    owner: Option<&str>,
    agent: &str,
    command: &str,
    args: &[String],
) -> Result<(), String> {
    let raw = match std::env::var(WORKSPACE_SNAPSHOT_ENV) {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => {
            return if std::env::var_os(buzz_core::local_workspace::PROJECT_POLICY_ENV).is_some()
                || std::env::var_os(buzz_core::local_workspace::PROJECT_GATE_ENV).is_some()
            {
                Err("project authority requires its scoped launch snapshot".into())
            } else {
                Ok(())
            }
        }
        Err(_) => return Err("channel workspace snapshot is not valid Unicode".into()),
    };
    let policy = buzz_agent::project_workspace::ProjectPolicy::from_env()?;
    let bridge = buzz_agent::codex_project::BridgeConfig::from_env()?;
    let gate = crate::project_gate::ProjectGateLaunch::from_env()?;
    if bridge.is_some() && policy.is_none() {
        return Err("Codex bridge requires native project authority".into());
    }
    if gate.is_some() && policy.is_some() {
        return Err("native project policy and harness project gate cannot both be set".into());
    }
    if let Some(policy) = policy {
        let snapshot = ChannelWorkspaceSnapshot::parse(
            &raw,
            relay,
            owner.ok_or("native project policy requires an owner")?,
        )?;
        if !cfg!(target_os = "macos")
            || std::path::Path::new(command)
                .file_name()
                .and_then(|name| name.to_str())
                != Some("buzz-agent")
            || if bridge.is_some() {
                args != ["--codex-project-bridge"]
            } else {
                !args.is_empty()
            }
            || policy.owner_pubkey != snapshot.owner_pubkey
            || policy.relay_url != snapshot.relay_url
            || policy.agent_pubkey != agent
        {
            return Err(
                "native project policy does not match runtime identity, scope, or launcher".into(),
            );
        }
        Ok(())
    } else if let Some(gate) = gate {
        let snapshot = ChannelWorkspaceSnapshot::parse(
            &raw,
            relay,
            owner.ok_or("project gate requires an owner")?,
        )?;
        validate_gate(&gate, &snapshot, agent, command, args)
    } else {
        validate_snapshot(&raw, relay, owner)
    }
}

fn validate_gate(
    gate: &crate::project_gate::ProjectGateLaunch,
    snapshot: &ChannelWorkspaceSnapshot,
    agent: &str,
    command: &str,
    args: &[String],
) -> Result<(), String> {
    if gate.policy.owner_pubkey != snapshot.owner_pubkey
        || gate.policy.relay_url != snapshot.relay_url
        || gate.policy.agent_pubkey != agent
    {
        return Err("project gate does not match runtime identity or scope".into());
    }
    if !snapshot.has_attached_projects() {
        return Err("project gate was supplied without any attached project".into());
    }
    if !args.is_empty() {
        return Err(
            "project gate does not support custom launcher arguments; the launcher was not replaced"
                .into(),
        );
    }
    if !gate_profile_matches_command(&gate.profile.harness, command) {
        return Err(format!(
            "project gate profile `{}` does not describe the configured agent command",
            gate.profile.harness
        ));
    }
    Ok(())
}

/// A runtime launched without project authority: attached projects on this device
/// are conversation-only for it. This is fail-closed (no project access), and the
/// desktop readiness check shows it for any team member on such a runtime.
fn validate_snapshot(raw: &str, relay: &str, owner: Option<&str>) -> Result<(), String> {
    let snapshot = ChannelWorkspaceSnapshot::parse(
        raw,
        relay,
        owner.ok_or("channel workspace snapshot requires an owner")?,
    )?;
    if snapshot.has_attached_projects() {
        tracing::info!(
            target: "acp::workspace",
            "this runtime has no project authority; attached project channels are conversation-only for it"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::local_workspace::ChannelWorkspaceBinding;

    #[test]
    fn local_workspace_accepts_verified_attestation_without_owner_environment() {
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "").unwrap();
        let resolved = resolve_owner(&agent.public_key(), Some(&tag), None);
        assert_eq!(resolved, Some(owner.public_key().to_hex()));
        let snapshot = ChannelWorkspaceSnapshot {
            version: 1,
            owner_pubkey: owner.public_key().to_hex(),
            relay_url: "wss://example.test".into(),
            bindings: vec![],
        };
        assert!(validate_snapshot(
            &serde_json::to_string(&snapshot).unwrap(),
            "wss://example.test",
            resolved.as_deref()
        )
        .is_ok());
        let foreign = nostr::Keys::generate();
        assert!(resolve_owner(&foreign.public_key(), Some(&tag), None).is_none());
    }

    #[test]
    fn local_workspace_launch_without_authority_is_conversation_only_but_scoped() {
        let mut snapshot = ChannelWorkspaceSnapshot {
            version: 1,
            owner_pubkey: "synthetic-owner".into(),
            relay_url: "wss://example.test".into(),
            bindings: vec![],
        };
        let verify = |snapshot: &ChannelWorkspaceSnapshot| {
            validate_snapshot(
                &serde_json::to_string(snapshot).unwrap(),
                "wss://example.test",
                Some("synthetic-owner"),
            )
        };
        assert!(verify(&snapshot).is_ok());
        snapshot.bindings.push(ChannelWorkspaceBinding {
            channel_id: uuid::Uuid::new_v4(),
            generation: 1,
            root: Some("/synthetic/project".into()),
        });
        // A saved folder never becomes execution permission: the launch is accepted
        // only because this runtime carries no authority for it.
        assert!(verify(&snapshot).is_ok());
        let raw = serde_json::to_string(&snapshot).unwrap();
        assert!(validate_snapshot(&raw, "wss://other.test", Some("synthetic-owner")).is_err());
        assert!(validate_snapshot(&raw, "wss://example.test", None).is_err());
        assert!(validate_snapshot("{}", "wss://example.test", Some("synthetic-owner")).is_err());
    }

    #[test]
    fn gate_launch_requires_matching_identity_scope_and_adapter() {
        use buzz_core::local_workspace::{Confinement, ProjectGateProfile};
        let gate = crate::project_gate::ProjectGateLaunch {
            policy: buzz_agent::project_workspace::ProjectPolicy {
                database: "/synthetic/archive.db".into(),
                owner_pubkey: "owner".into(),
                relay_url: "wss://example.test".into(),
                agent_pubkey: "agent".into(),
                protected_roots: vec![],
                agent_store: "/synthetic/agents.json".into(),
            },
            profile: ProjectGateProfile {
                harness: "claude".into(),
                gate_mode: Some("default".into()),
                write_confinement: Confinement::HarnessGate,
                read_confinement: Confinement::Unconfined,
                command_confinement: Confinement::HarnessGate,
                network_blocked: false,
                evidence: "synthetic".into(),
            },
            task_mcp_command: "/synthetic/buzz-agent".into(),
        };
        let mut snapshot = ChannelWorkspaceSnapshot {
            version: 1,
            owner_pubkey: "owner".into(),
            relay_url: "wss://example.test".into(),
            bindings: vec![ChannelWorkspaceBinding {
                channel_id: uuid::Uuid::new_v4(),
                generation: 1,
                root: Some("/synthetic/project".into()),
            }],
        };
        assert!(validate_gate(&gate, &snapshot, "agent", "claude-agent-acp", &[]).is_ok());
        assert!(validate_gate(&gate, &snapshot, "other", "claude-agent-acp", &[]).is_err());
        assert!(validate_gate(&gate, &snapshot, "agent", "codex-acp", &[]).is_err());
        assert!(validate_gate(&gate, &snapshot, "agent", "goose", &[]).is_err());
        assert!(validate_gate(
            &gate,
            &snapshot,
            "agent",
            "claude-agent-acp",
            &["--x".into()]
        )
        .is_err());
        snapshot.bindings[0].root = None;
        assert!(validate_gate(&gate, &snapshot, "agent", "claude-agent-acp", &[]).is_err());
        assert!(gate_profile_matches_command("codex", "/opt/bin/codex-acp"));
        assert!(!gate_profile_matches_command("buzz-agent", "buzz-agent"));
    }
}
