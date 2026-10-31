//! Native project sessions use authoritative local grants and fixed tools, never MCP launchers.

use buzz_core::local_workspace::{ProjectSessionScope, PROJECT_POLICY_ENV};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod approvals;
mod delegation;
pub mod edits;
pub(crate) mod files;
mod helper;
pub mod task_mcp;
pub mod task_tools;
pub mod tasks;
pub mod teams;

/// Desktop-owned locator; contains no keys and must stay device-local.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPolicy {
    /// Existing native archive database.
    pub database: String,
    /// Authenticated device owner.
    pub owner_pubkey: String,
    /// Canonical relay scope.
    pub relay_url: String,
    /// Existing agent this process represents.
    pub agent_pubkey: String,
    /// Native identity and authority directories that may never be a workspace.
    pub protected_roots: Vec<String>,
    /// Native existing-agent metadata store; secret fields are ignored, never hydrated.
    pub agent_store: String,
}

#[derive(Deserialize)]
struct Team {
    generation: i64,
    binding_generation: i64,
    coordinator_pubkey: Option<String>,
    members: Vec<Member>,
    paused: bool,
    #[serde(default)]
    approved_checks: Vec<String>,
}

#[derive(Deserialize)]
struct Member {
    pubkey: String,
    #[serde(default)]
    role: String,
    capabilities: Vec<String>,
}

/// Effective project authority of one agent in one channel, resolved from the
/// device-local store for a harness-gated session. It is a snapshot fenced by
/// both generations inside `scope`; any change invalidates the session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionGrants {
    /// Fenced session scope (owner, relay, agent, channel, root, generations).
    pub scope: ProjectSessionScope,
    /// This agent's project role label.
    pub role: String,
    /// This agent's granted capabilities.
    pub capabilities: Vec<String>,
    /// Owner-approved exact check command lines for `run_checks` holders.
    pub approved_checks: Vec<String>,
}

impl SessionGrants {
    /// Whether the agent holds one capability.
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|grant| grant == capability)
    }
}

impl ProjectPolicy {
    /// Parse only explicitly supplied native configuration; malformed values fail closed.
    pub fn from_env() -> Result<Option<Self>, String> {
        match std::env::var(PROJECT_POLICY_ENV) {
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err("native project policy is not Unicode".into()),
            Ok(raw) => Self::parse(&raw).map(Some),
        }
    }

    /// Parse one serialized policy; shared by the native and harness-gated launch paths.
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw.len() > 16384 {
            return Err("native project policy exceeds size limit".into());
        }
        let policy: Self =
            serde_json::from_str(raw).map_err(|_| "invalid native project policy")?;
        if !Path::new(&policy.database).is_absolute() {
            return Err("native policy database must be absolute".into());
        }
        Ok(policy)
    }

    /// Resolve this agent's full grant set for a channel, or `None` when the channel
    /// has no attached project. Missing membership, a paused team, or stale
    /// generations are errors, never an empty grant.
    pub fn session_grants(&self, channel: &str) -> Result<Option<SessionGrants>, String> {
        let mut conn = self.open()?;
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        let Some(scope) = self.resolve(&transaction, channel, "read")? else {
            return Ok(None);
        };
        let raw: String = transaction.query_row("SELECT snapshot FROM channel_project_teams WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3",
            params![self.owner_pubkey,self.relay_url,channel],|row|row.get(0)).map_err(|_|"project team is not configured")?;
        let team: Team = serde_json::from_str(&raw).map_err(|_| "invalid project authority")?;
        let member = team
            .members
            .iter()
            .find(|member| member.pubkey == self.agent_pubkey)
            .ok_or("agent is not on this project team")?;
        Ok(Some(SessionGrants {
            scope,
            role: member.role.clone(),
            capabilities: member.capabilities.clone(),
            approved_checks: team.approved_checks.clone(),
        }))
    }

    /// Durably record a gate decision that needs the owner, deduplicated while pending.
    /// Returns the request id. Never grants anything by itself.
    pub fn record_approval_request(
        &self,
        scope: &ProjectSessionScope,
        request: approvals::NewApprovalRequest<'_>,
    ) -> Result<String, String> {
        let mut conn = self.open()?;
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let id = approvals::record(&transaction, scope, request)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(id)
    }

    /// The owner's decision on one recorded request, if any has been made:
    /// `pending`, `approved` or `denied`, or `None` when the request is gone.
    pub fn approval_state(&self, request_id: &str) -> Result<Option<String>, String> {
        let conn = self.open()?;
        approvals::state(&conn, &self.owner_pubkey, &self.relay_url, request_id)
    }

    pub(super) fn open(&self) -> Result<Connection, String> {
        let conn = Connection::open_with_flags(&self.database, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| {
            "native project authority database unavailable; restore the local profile".to_string()
        })?;
        conn.busy_timeout(std::time::Duration::from_millis(250))
            .map_err(|error| error.to_string())?;
        Ok(conn)
    }

    pub(super) fn resolve(
        &self,
        conn: &Connection,
        channel: &str,
        capability: &str,
    ) -> Result<Option<ProjectSessionScope>, String> {
        let binding: Option<(i64,Option<String>)> = conn.query_row(
            "SELECT generation,root FROM channel_workspaces WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3",
            params![self.owner_pubkey,self.relay_url,channel],|row| Ok((row.get(0)?,row.get(1)?)))
            .optional().map_err(|error|error.to_string())?;
        let Some((generation, Some(root))) = binding else {
            return Ok(None);
        };
        let canonical =
            std::fs::canonicalize(&root).map_err(|_| "project directory unavailable")?;
        if canonical!=Path::new(&root) || Path::new(&root).components().any(|part|matches!(part,std::path::Component::Normal(name) if name.to_string_lossy().starts_with('.'))) {
            return Err("project root must be canonical and outside hidden identity directories".into());
        }
        let database =
            std::fs::canonicalize(&self.database).map_err(|_| "native authority unavailable")?;
        if database.starts_with(&canonical) {
            return Err("project may not contain its authority database".into());
        }
        let agent_store = std::fs::canonicalize(&self.agent_store)
            .map_err(|_| "existing-agent authority store unavailable")?;
        if !Path::new(&self.agent_store).is_absolute() || agent_store.starts_with(&canonical) {
            return Err("project may not contain the existing-agent authority store".into());
        }
        for protected in &self.protected_roots {
            if !Path::new(protected).is_absolute() {
                return Err("invalid native protected directory".into());
            }
            let protected = std::fs::canonicalize(protected).map_err(|_| {
                "protected identity directory unavailable; native authority cannot be verified"
            })?;
            if !protected.is_absolute()
                || protected.starts_with(&canonical)
                || canonical.starts_with(&protected)
            {
                return Err(
                    "project overlaps a protected native identity or authority directory".into(),
                );
            }
        }
        let raw: String = conn.query_row("SELECT snapshot FROM channel_project_teams WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3",
            params![self.owner_pubkey,self.relay_url,channel],|row|row.get(0)).map_err(|_|"project team is not configured")?;
        let team: Team = serde_json::from_str(&raw).map_err(|_| "invalid project authority")?;
        if team.paused
            || team.coordinator_pubkey.is_none()
            || team.binding_generation != generation
            || team.generation < 1
            || !team.members.iter().any(|member| {
                member.pubkey == self.agent_pubkey
                    && member.capabilities.iter().any(|grant| grant == capability)
            })
        {
            return Err("project authority missing, paused, stale, or insufficient".into());
        }
        Ok(Some(ProjectSessionScope {
            owner_pubkey: self.owner_pubkey.clone(),
            relay_url: self.relay_url.clone(),
            agent_pubkey: self.agent_pubkey.clone(),
            channel_id: channel.parse().map_err(|_| "invalid project channel")?,
            root,
            binding_generation: generation,
            team_generation: team.generation,
        }))
    }

    /// Resolve a channel against the native database rather than ambient working directory.
    pub fn session_scope(&self, channel: &str) -> Result<Option<ProjectSessionScope>, String> {
        let mut conn = self.open()?;
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        self.resolve(&transaction, channel, "read")
    }

    /// Keep a native database read transaction alive while an authorized operation executes.
    pub fn with_authority<T>(
        &self,
        scope: &ProjectSessionScope,
        capability: &str,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut conn = self.open()?;
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let current = self
            .resolve(&transaction, &scope.channel_id.to_string(), capability)?
            .ok_or("project was detached")?;
        if &current != scope {
            return Err("project session is stale; create a new authorized session".into());
        }
        let result = operation()?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(result)
    }
}

/// One native session, with policy verified before each filesystem operation.
#[derive(Clone)]
pub(crate) struct ProjectWorkspace {
    pub policy: ProjectPolicy,
    pub scope: ProjectSessionScope,
    pub operation_prefix: String,
}

impl ProjectWorkspace {
    pub fn validate(&self) -> Result<(), String> {
        self.policy.with_authority(&self.scope, "read", || Ok(()))
    }

    pub async fn read(&self, path: String) -> Result<String, String> {
        let workspace = self.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            workspace
                .policy
                .with_authority(&workspace.scope, "read", || {
                    runtime.block_on(helper::run(&workspace.scope.root, &path, None))
                })
        })
        .await
        .map_err(|error| error.to_string())?
    }

    pub async fn edit(
        &self,
        request_id: String,
        path: String,
        expected_sha256: Option<String>,
        content: String,
    ) -> Result<String, String> {
        let workspace = self.clone();
        let runtime = tokio::runtime::Handle::current();
        let operation_id = format!("{}:{request_id}", self.operation_prefix);
        tokio::task::spawn_blocking(move || {
            edits::replace(
                &workspace.policy,
                &workspace.scope,
                &operation_id,
                &path,
                expected_sha256.as_deref().unwrap_or("absent"),
                &content,
                || {
                    runtime.block_on(helper::run(
                        &workspace.scope.root,
                        &path,
                        Some(helper::Replacement {
                            expected_sha256: expected_sha256.clone(),
                            content: content.clone(),
                        }),
                    ))
                },
            )
        })
        .await
        .map_err(|error| error.to_string())?
    }

    pub async fn team(&self, next: Option<Vec<teams::ProjectMember>>) -> Result<String, String> {
        let workspace = self.clone();
        tokio::task::spawn_blocking(move || {
            delegation::operate(&workspace.policy, &workspace.scope, next)
        })
        .await
        .map_err(|error| error.to_string())?
    }
}

/// Run the bounded file-helper entry point before any provider configuration or credentials.
pub fn run_file_helper_if_requested() -> Option<Result<(), String>> {
    helper::serve_if_requested()
}
