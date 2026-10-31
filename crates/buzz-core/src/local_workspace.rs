//! Device-local workspace contracts. These values must never be published as events.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// Maximum number of channel bindings carried by a local runtime.
pub const MAX_CHANNEL_WORKSPACES: usize = 256;
/// Maximum serialized size of a local runtime workspace snapshot.
pub const MAX_WORKSPACE_SNAPSHOT_BYTES: usize = 1024 * 1024;
/// Desktop-owned environment key for the immutable launch snapshot.
pub const WORKSPACE_SNAPSHOT_ENV: &str = "BUZZ_ACP_CHANNEL_WORKSPACES";

/// Device-local policy locator for the native bounded runtime; never relay metadata.
pub const PROJECT_POLICY_ENV: &str = "BUZZ_NATIVE_PROJECT_POLICY";

/// Desktop-owned environment key for a harness-gated project launch. Carries a
/// [`ProjectGateProfile`] plus the same device-local policy locator the native
/// runtime uses; it is only emitted for runtimes whose enforcement profile the
/// desktop catalog declares, never inferred from a runtime name.
pub const PROJECT_GATE_ENV: &str = "BUZZ_ACP_PROJECT_GATE";

/// How one boundary is enforced for project sessions on a given runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confinement {
    /// An operating-system sandbox (Seatbelt / the runtime's own sandbox) enforces the boundary.
    OsSandbox,
    /// The harness routes each affected tool call through `session/request_permission`
    /// and the Buzz harness answers from the project policy. This is a gate, not a sandbox.
    HarnessGate,
    /// Nothing enforces this boundary on this runtime; it must be shown as unconfined.
    Unconfined,
}

/// Verified enforcement facts for one runtime, declared by the desktop runtime
/// catalog (the single source of harness capability facts) and echoed to the
/// harness at launch so both sides describe the same boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectGateProfile {
    /// Catalog runtime id, e.g. `claude` or `codex`.
    pub harness: String,
    /// ACP `mode` config value the harness must accept for project sessions.
    /// `None` means the runtime has no session mode to pin.
    pub gate_mode: Option<String>,
    /// Confinement of writes to the project root.
    pub write_confinement: Confinement,
    /// Confinement of reads to the project root.
    pub read_confinement: Confinement,
    /// Confinement of shell/command execution to the approved check list.
    pub command_confinement: Confinement,
    /// Whether outbound network access from project sessions is blocked.
    pub network_blocked: bool,
    /// Short human-readable evidence note shown in readiness diagnostics.
    pub evidence: String,
}

impl ProjectGateProfile {
    /// A profile only supports project execution when writes are confined by
    /// something stronger than the model's good will.
    pub fn supports_project_execution(&self) -> bool {
        self.write_confinement != Confinement::Unconfined
    }

    /// Whether this profile can honour a member's grant set exactly. A runtime whose
    /// own sandbox permits writes inside the project cannot withhold them per edit,
    /// so it may only run for members who were granted `edit`; likewise a runtime
    /// that executes commands inside its sandbox without per-command approval may
    /// only run for members granted `run_checks`. Anything else would advertise a
    /// boundary the runtime does not enforce.
    pub fn enforceable_for(&self, capabilities: &[String]) -> Result<(), String> {
        let has = |capability: &str| capabilities.iter().any(|grant| grant == capability);
        if !self.supports_project_execution() {
            return Err(format!(
                "runtime `{}` does not confine writes to the project",
                self.harness
            ));
        }
        if self.write_confinement == Confinement::OsSandbox && !has("edit") {
            return Err(format!(
                "runtime `{}` confines writes with its own sandbox but cannot withhold them per edit; grant edit or choose a runtime with a permission gate",
                self.harness
            ));
        }
        if self.command_confinement == Confinement::Unconfined && !has("run_checks") {
            return Err(format!(
                "runtime `{}` runs commands inside its sandbox without per-command approval; grant run_checks or choose a runtime with a permission gate",
                self.harness
            ));
        }
        Ok(())
    }
}

/// Immutable authority captured when creating one native project session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSessionScope {
    /// Current device owner.
    pub owner_pubkey: String,
    /// Canonical community identity.
    pub relay_url: String,
    /// Existing participating agent.
    pub agent_pubkey: String,
    /// Session channel, never inferred from a prompt.
    pub channel_id: Uuid,
    /// Exact canonical working directory.
    pub root: String,
    /// Binding revision captured by the session.
    pub binding_generation: i64,
    /// Team revision captured by the session.
    pub team_generation: i64,
}

/// Proof that a runtime launch carries no project authority for its agent: either the
/// snapshot has no attached projects, or the agent is on no bound channel's team. It is
/// never a sandbox attestation.
#[derive(Debug)]
pub struct ConversationOnlyLaunch(());

/// One device-local channel binding, including a generation-preserving detach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelWorkspaceBinding {
    /// Channel whose sessions use this binding.
    pub channel_id: Uuid,
    /// Monotonic revision, including detach operations.
    pub generation: i64,
    /// Canonical local directory; absent means conversation only.
    pub root: Option<String>,
}

/// Immutable configuration for one owner and community at runtime launch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelWorkspaceSnapshot {
    /// Schema version; unknown versions fail closed.
    pub version: u32,
    /// Canonical runtime relay identity.
    pub relay_url: String,
    /// Owner's public identity, never a private key.
    pub owner_pubkey: String,
    /// Channel bindings on this device; paths do not travel through the relay.
    pub bindings: Vec<ChannelWorkspaceBinding>,
}

impl ChannelWorkspaceSnapshot {
    /// Whether any channel on this device has a local project attached.
    pub fn has_attached_projects(&self) -> bool {
        self.bindings.iter().any(|binding| binding.root.is_some())
    }

    /// Fail closed until a runtime can attest and enforce project-scoped access.
    pub fn require_supported_execution(&self) -> Result<ConversationOnlyLaunch, String> {
        if self.has_attached_projects() {
            return Err("channel project execution is unsupported: no verified workspace permission enforcement; detach the local binding to resume conversation-only runtimes".into());
        }
        Ok(ConversationOnlyLaunch(()))
    }

    /// Prove a launch is conversation-only for one agent: attached projects on other
    /// channels do not matter unless this agent was granted project authority somewhere.
    /// `bound_team_channels` must be the channels with an attached root whose team
    /// lists the agent; the caller reads it from the device-local authority store.
    pub fn conversation_only_for_agent(
        &self,
        bound_team_channels: &[Uuid],
    ) -> Result<ConversationOnlyLaunch, String> {
        if bound_team_channels.is_empty() {
            return Ok(ConversationOnlyLaunch(()));
        }
        Err(format!(
            "agent is on the project team of {} attached channel(s) but its runtime has no verified project enforcement; choose a supported runtime or release it from the team",
            bound_team_channels.len()
        ))
    }

    /// Parse a bounded snapshot and reject ambiguous or cross-community input.
    pub fn parse(raw: &str, relay_url: &str, owner_pubkey: &str) -> Result<Self, String> {
        if raw.len() > MAX_WORKSPACE_SNAPSHOT_BYTES {
            return Err("channel workspace snapshot exceeds size limit".into());
        }
        let snapshot: Self = serde_json::from_str(raw)
            .map_err(|_| "invalid channel workspace snapshot".to_string())?;
        let relay =
            crate::relay::normalize_relay_url(relay_url).map_err(|error| error.to_string())?;
        if snapshot.version != 1
            || snapshot.relay_url != relay
            || snapshot.owner_pubkey != owner_pubkey
            || snapshot.bindings.len() > MAX_CHANNEL_WORKSPACES
        {
            return Err("channel workspace snapshot scope or version mismatch".into());
        }
        let mut channels = HashSet::new();
        for binding in &snapshot.bindings {
            if binding.generation < 1 || !channels.insert(binding.channel_id) {
                return Err("duplicate channel or invalid workspace generation".into());
            }
            if let Some(root) = &binding.root {
                if root.is_empty() || root.len() > 4096 || root.contains('\0') {
                    return Err("invalid local workspace path".into());
                }
            }
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_snapshot_rejects_wrong_scope_version_and_duplicates() {
        let mut snapshot = ChannelWorkspaceSnapshot {
            version: 1,
            relay_url: "wss://example.test".into(),
            owner_pubkey: "owner".into(),
            bindings: vec![ChannelWorkspaceBinding {
                channel_id: Uuid::new_v4(),
                generation: 1,
                root: Some("/synthetic/project".into()),
            }],
        };
        let parse = |snapshot: &ChannelWorkspaceSnapshot, relay, owner| {
            ChannelWorkspaceSnapshot::parse(&serde_json::to_string(snapshot).unwrap(), relay, owner)
        };
        assert!(parse(&snapshot, "wss://example.test/", "owner").is_ok());
        assert!(snapshot.require_supported_execution().is_err());
        let mut detached = snapshot.clone();
        detached.bindings[0].root = None;
        assert!(detached.require_supported_execution().is_ok());
        assert!(parse(&snapshot, "wss://other.test", "owner").is_err());
        assert!(parse(&snapshot, "wss://example.test", "other").is_err());
        snapshot.version = 2;
        assert!(parse(&snapshot, "wss://example.test", "owner").is_err());
        snapshot.version = 1;
        snapshot.bindings.push(snapshot.bindings[0].clone());
        assert!(parse(&snapshot, "wss://example.test", "owner").is_err());
        assert!(ChannelWorkspaceSnapshot::parse("{}", "wss://example.test", "owner").is_err());
    }

    #[test]
    fn conversation_only_launch_depends_on_agent_membership_not_other_channels() {
        let snapshot = ChannelWorkspaceSnapshot {
            version: 1,
            relay_url: "wss://example.test".into(),
            owner_pubkey: "owner".into(),
            bindings: vec![ChannelWorkspaceBinding {
                channel_id: Uuid::new_v4(),
                generation: 1,
                root: Some("/synthetic/project".into()),
            }],
        };
        assert!(snapshot.has_attached_projects());
        assert!(snapshot.conversation_only_for_agent(&[]).is_ok());
        let error = snapshot
            .conversation_only_for_agent(&[Uuid::new_v4()])
            .unwrap_err();
        assert!(error.contains("no verified project enforcement"));
    }

    #[test]
    fn gate_profile_requires_confined_writes() {
        let mut profile = ProjectGateProfile {
            harness: "claude".into(),
            gate_mode: Some("default".into()),
            write_confinement: Confinement::HarnessGate,
            read_confinement: Confinement::Unconfined,
            command_confinement: Confinement::HarnessGate,
            network_blocked: false,
            evidence: "synthetic".into(),
        };
        assert!(profile.supports_project_execution());
        assert!(profile.enforceable_for(&["read".into()]).is_ok());
        let sandboxed = ProjectGateProfile {
            write_confinement: Confinement::OsSandbox,
            command_confinement: Confinement::Unconfined,
            ..profile.clone()
        };
        assert!(sandboxed.enforceable_for(&["read".into()]).is_err());
        assert!(sandboxed
            .enforceable_for(&["read".into(), "edit".into()])
            .is_err());
        assert!(sandboxed
            .enforceable_for(&["read".into(), "edit".into(), "run_checks".into()])
            .is_ok());
        profile.write_confinement = Confinement::Unconfined;
        assert!(!profile.supports_project_execution());
        assert!(profile.enforceable_for(&["edit".into()]).is_err());
        let raw = serde_json::to_string(&profile).unwrap();
        assert_eq!(
            serde_json::from_str::<ProjectGateProfile>(&raw).unwrap(),
            profile
        );
    }
}
