//! Project-execution enforcement facts, projected from the runtime catalog.
//!
//! The catalog (`KnownAcpRuntime::project_enforcement`) is the only place these
//! facts are declared. This module turns them into the shapes the launch path
//! (`ProjectGateProfile`, echoed to the harness) and the UI/readiness check
//! (`ProjectEnforcementSummary`) consume.

use super::discovery::runtime_metadata::ProjectEnforcementDecl;
use super::types::ProjectEnforcementSummary;
use super::KnownAcpRuntime;
use buzz_core_pkg::local_workspace::ProjectGateProfile;

/// The gate profile for a harness-gated runtime, or `None` for native/unverified ones.
pub(crate) fn gate_profile(runtime: &KnownAcpRuntime) -> Option<ProjectGateProfile> {
    match &runtime.project_enforcement {
        ProjectEnforcementDecl::Gate {
            gate_mode,
            write,
            read,
            command,
            network_blocked,
            evidence,
        } => Some(ProjectGateProfile {
            harness: runtime.id.to_string(),
            gate_mode: Some((*gate_mode).to_string()),
            write_confinement: *write,
            read_confinement: *read,
            command_confinement: *command,
            network_blocked: *network_blocked,
            evidence: (*evidence).to_string(),
        }),
        _ => None,
    }
}

/// Summary for a catalog runtime; `None` (presets, custom launchers, unknown
/// commands) is always unverified.
pub(crate) fn summary_for_runtime(runtime: Option<&KnownAcpRuntime>) -> ProjectEnforcementSummary {
    match runtime.map(|runtime| &runtime.project_enforcement) {
        Some(ProjectEnforcementDecl::NativeSandbox) => ProjectEnforcementSummary::NativeSandbox,
        Some(ProjectEnforcementDecl::Gate { .. }) => ProjectEnforcementSummary::Gate {
            profile: runtime
                .and_then(gate_profile)
                .expect("gate declaration always yields a profile"),
        },
        Some(ProjectEnforcementDecl::Unverified { reason }) => {
            ProjectEnforcementSummary::Unverified {
                reason: (*reason).to_string(),
            }
        }
        None => ProjectEnforcementSummary::Unverified {
            reason:
                "this launcher has no verified project enforcement profile in the runtime catalog"
                    .to_string(),
        },
    }
}

/// Summary keyed by a saved runtime id (`ManagedAgentRecord::runtime`).
pub(crate) fn summary_for_runtime_id(runtime_id: Option<&str>) -> ProjectEnforcementSummary {
    summary_for_runtime(runtime_id.and_then(super::known_acp_runtime_exact))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core_pkg::local_workspace::Confinement;

    #[test]
    fn catalog_declarations_project_to_summaries() {
        assert_eq!(
            summary_for_runtime_id(Some("buzz-agent")),
            ProjectEnforcementSummary::NativeSandbox
        );
        match summary_for_runtime_id(Some("claude")) {
            ProjectEnforcementSummary::Gate { profile } => {
                assert_eq!(profile.harness, "claude");
                assert_eq!(profile.gate_mode.as_deref(), Some("default"));
                assert_eq!(profile.write_confinement, Confinement::HarnessGate);
                assert!(profile.enforceable_for(&["read".into()]).is_ok());
            }
            other => panic!("claude must be gated: {other:?}"),
        }
        match summary_for_runtime_id(Some("codex")) {
            ProjectEnforcementSummary::Gate { profile } => {
                assert_eq!(profile.gate_mode.as_deref(), Some("read-only"));
                assert_eq!(profile.write_confinement, Confinement::OsSandbox);
                assert!(profile.network_blocked);
                assert!(profile.enforceable_for(&["read".into()]).is_err());
                assert!(profile
                    .enforceable_for(&["read".into(), "edit".into(), "run_checks".into()])
                    .is_ok());
            }
            other => panic!("codex must be gated: {other:?}"),
        }
        assert!(matches!(
            summary_for_runtime_id(Some("goose")),
            ProjectEnforcementSummary::Unverified { .. }
        ));
        assert!(matches!(
            summary_for_runtime_id(Some("pi")),
            ProjectEnforcementSummary::Unverified { .. }
        ));
        assert!(matches!(
            summary_for_runtime_id(None),
            ProjectEnforcementSummary::Unverified { .. }
        ));
    }
}
