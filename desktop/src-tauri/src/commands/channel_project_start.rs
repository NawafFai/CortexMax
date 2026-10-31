use super::channel_workspaces::{require_idle_runtimes, scope};
use crate::{
    app_state::AppState,
    archive::{channel_workspaces, project_teams},
    managed_agents,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

/// Machine-readable recovery reason, never an inferred readiness certificate.
#[derive(Serialize)]
pub struct ProjectStartBlocker {
    /// Stable recovery code.
    pub code: String,
    /// Actionable explanation without secrets.
    pub message: String,
}

/// Evidence of a locally started process, not of provider execution.
#[derive(Serialize)]
pub struct ProjectStartReceipt {
    /// Native child process identifier.
    pub pid: u32,
    /// Unix seconds when this process was launched.
    pub started_at: u64,
}

/// Generation-fenced coordinator startup outcome.
#[derive(Serialize)]
pub struct ProjectStartOutcome {
    /// `started` or `blocked`.
    pub status: String,
    /// Explicitly selected native transport, or `unsupported`.
    pub transport: String,
    /// Observed binding revision.
    pub binding_generation: i64,
    /// Observed team revision.
    pub team_generation: i64,
    /// Existing coordinator identity, never newly created.
    pub coordinator_pubkey: String,
    /// Owner must select the reasoning-only bridge explicitly.
    pub requires_explicit_opt_in: bool,
    /// A separate scoped login is required; credentials are never copied.
    pub authentication_required: bool,
    /// Recovery information.
    pub blockers: Vec<ProjectStartBlocker>,
    /// Native process-start evidence only.
    pub runtime_receipt: Option<ProjectStartReceipt>,
    /// Always false: process startup alone is not execution readiness.
    pub execution_ready: bool,
}

impl ProjectStartOutcome {
    fn block(mut self, code: &str, message: impl Into<String>) -> Self {
        self.blockers.push(ProjectStartBlocker {
            code: code.into(),
            message: message.into(),
        });
        self
    }
}

fn authority(
    conn: &rusqlite::Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    binding_generation: i64,
    team_generation: i64,
    coordinator: &str,
) -> Result<(), String> {
    crate::archive::workspace_provider_attempts::require_released(conn, relay)?;
    crate::archive::project_channel_creations::require_settled(conn, owner, relay)?;
    buzz_agent_pkg::project_workspace::edits::require_reconciled(conn, relay)?;
    let binding = channel_workspaces::list(conn, owner, relay)?
        .into_iter()
        .find(|binding| binding.channel_id == channel)
        .ok_or("attach a project first")?;
    if binding.generation != binding_generation {
        return Err("project binding changed; reload before starting".into());
    }
    channel_workspaces::validate_root(binding.root.as_deref().ok_or("attach a project first")?)?;
    let team = project_teams::load(conn, owner, relay, channel)?;
    if team.coordinator_pubkey.as_deref() != Some(coordinator) {
        return Err("project coordinator changed; reload before starting".into());
    }
    project_teams::authorize(
        &team,
        binding_generation,
        team_generation,
        coordinator,
        "read",
    )
}

fn transport(command: &str, args: &[String], harness: &str, local: bool) -> &'static str {
    if !cfg!(target_os = "macos") || !local || !args.is_empty() || harness != "buzz-acp" {
        return "unsupported";
    }
    match command {
        "buzz-agent" => "native",
        "codex-acp" => "codex_reasoning_broker_v1",
        _ => "unsupported",
    }
}

fn codex_descriptor(
    app: &AppHandle,
    owner: &str,
    relay: &str,
    pubkey: &str,
    model: String,
) -> Result<buzz_agent_pkg::codex_project::BridgeConfig, String> {
    use sha2::{Digest, Sha256};
    if std::env::consts::ARCH != "aarch64" {
        return Err("this Codex broker binary contract is verified only on Apple silicon".into());
    }
    let adapter = managed_agents::resolve_command("codex-acp")
        .ok_or("configured Codex adapter is unavailable")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let package = adapter
        .parent()
        .and_then(|parent| parent.parent())
        .ok_or("Codex adapter package root unavailable")?;
    let binary = package
        .join("node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex")
        .canonicalize()
        .map_err(|_| {
            "configured adapter's Codex binary is unavailable; no alternate runtime was selected"
        })?;
    let hash = hex::encode(Sha256::digest(
        format!("{owner}\n{relay}\n{pubkey}").as_bytes(),
    ));
    let data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let profiles = data.join("codex-project-profiles");
    let home = profiles.join(hash);
    for directory in [&profiles, &home] {
        match std::fs::create_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        if directory
            .canonicalize()
            .map_err(|error| error.to_string())?
            != *directory
            || !directory.is_dir()
        {
            return Err("scoped Codex profile may not traverse a symlink".into());
        }
    }
    if home.canonicalize().map_err(|error| error.to_string())? != home {
        return Err("scoped Codex profile may not traverse a symlink".into());
    }
    Ok(buzz_agent_pkg::codex_project::BridgeConfig {
        binary: binary
            .to_str()
            .ok_or("Codex binary path is not Unicode")?
            .into(),
        home: home
            .to_str()
            .ok_or("Codex profile path is not Unicode")?
            .into(),
        model,
        synthetic_service: None,
    })
}

/// Start only an existing, locally managed coordinator after current native authority checks.
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri's stable IPC has independently named scope and CAS fields"
)]
pub async fn start_channel_project_coordinator(
    app: AppHandle,
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_owner: String,
    expected_relay: String,
    expected_binding_generation: i64,
    expected_team_generation: i64,
    coordinator_pubkey: String,
    explicit_bridge_opt_in: bool,
) -> Result<ProjectStartOutcome, String> {
    let changes = state
        .channel_workspace_change_lock
        .clone()
        .read_owned()
        .await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let events=crate::relay::query_relay(&state,&[serde_json::json!({"kinds":[buzz_core_pkg::kind::KIND_NIP29_GROUP_ADMINS],"#d":[channel_id.to_string()],"limit":1})]).await?;
    if !events.iter().any(|event| {
        event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().is_some_and(|value| value == "p")
                && values.get(1) == Some(&owner)
                && values.iter().skip(2).any(|role| role == "owner")
        })
    }) {
        return Err("current channel ownership could not be verified".into());
    }
    tauri::async_runtime::spawn(async move {
        let _changes=changes;
        let state=app.state::<AppState>();
        let check_owner=owner.clone(); let check_relay=relay.clone(); let check_coordinator=coordinator_pubkey.clone();
        state.archive_db.with_conn(move |conn|authority(conn,&check_owner,&check_relay,channel_id,expected_binding_generation,expected_team_generation,&check_coordinator)).await?;
        let prepare_app=app.clone(); let prepare_owner=owner.clone(); let prepare_coordinator=coordinator_pubkey.clone();
        let mut outcome=ProjectStartOutcome {status:"blocked".into(),transport:"unsupported".into(),binding_generation:expected_binding_generation,team_generation:expected_team_generation,coordinator_pubkey:coordinator_pubkey.clone(),requires_explicit_opt_in:false,authentication_required:false,blockers:vec![],runtime_receipt:None,execution_ready:false};
        let prepared=tauri::async_runtime::spawn_blocking(move || {
            let state=prepare_app.state::<AppState>();
            let _store=state.managed_agents_store_lock.lock().map_err(|error|error.to_string())?;
            let candidates=managed_agents::storage::load_project_agent_metadata(&prepare_app,&prepare_owner)?;
            if !candidates.iter().any(|agent|agent.pubkey==prepare_coordinator && agent.local) { return Ok(None); }
            let records=managed_agents::storage::load_project_launch_records(&prepare_app)?;
            let record=records.into_iter().find(|record|record.pubkey==prepare_coordinator).ok_or("managed coordinator disappeared")?;
            let personas=managed_agents::load_personas(&prepare_app)?;
            let global=managed_agents::load_global_agent_config(&prepare_app)?;
            let descriptor=managed_agents::resolve_effective_harness_descriptor(&record,&personas,&global).map_err(|_|"configured coordinator launcher could not be resolved")?;
            let selected=transport(&descriptor.command,&descriptor.args,&record.acp_command,record.backend==managed_agents::BackendKind::Local);
            let model=managed_agents::effective_config::resolve_effective_config(&record,&personas,&global).require_resolved()?.model.value;
            Ok::<_,String>(Some((record,selected,model)))
        }).await.map_err(|error|error.to_string())??;
        let Some((record,selected,model))=prepared else { return Ok(outcome.block("coordinator_not_managed_locally","The selected identity is not an owner-attested local managed agent in this profile. No identity was adopted, copied, or created.")); };
        outcome.transport=selected.into();
        if selected=="unsupported" { return Ok(outcome.block("unsupported_project_transport","The configured runtime or constrained launcher does not implement this project boundary; it was not replaced.")); }
        let bridge=if selected=="codex_reasoning_broker_v1" {
            if !explicit_bridge_opt_in { outcome.requires_explicit_opt_in=true; return Ok(outcome.block("explicit_bridge_opt_in_required","Explicitly opt in to the reasoning-only Codex bridge. Existing identity/model stay unchanged; native project tools alone access project files, using a separately authenticated profile.")); }
            let Some(model)=model else { return Ok(outcome.block("model_not_configured","Choose an explicit model on the existing persona; no fallback model was selected.")); };
            let config=match codex_descriptor(&app,&owner,&relay,&coordinator_pubkey,model) { Ok(config)=>config,Err(error)=>return Ok(outcome.block("codex_bridge_unavailable",error)) };
            let auth_result=buzz_agent_pkg::codex_project::probe_scoped_authentication(&config).await;
            scope(&state,&expected_relay,&expected_owner)?;
            let check_owner=owner.clone(); let check_relay=relay.clone(); let check_coordinator=coordinator_pubkey.clone();
            state.archive_db.with_conn(move |conn|authority(conn,&check_owner,&check_relay,channel_id,expected_binding_generation,expected_team_generation,&check_coordinator)).await?;
            match auth_result {
                Ok(true)=>{},
                Ok(false)=>{ outcome.authentication_required=true; return Ok(outcome.block("scoped_authentication_required","The dedicated Codex project profile is not authenticated. Separate owner-approved login is required; ambient credentials are never borrowed or copied.")); },
                Err(error)=>return Ok(outcome.block("scoped_authentication_probe_failed",error)),
            }
            Some(config)
        } else { None };
        let check_owner=owner.clone(); let check_relay=relay.clone(); let check_coordinator=coordinator_pubkey.clone();
        state.archive_db.with_conn(move |conn|authority(conn,&check_owner,&check_relay,channel_id,expected_binding_generation,expected_team_generation,&check_coordinator)).await?;
        tauri::async_runtime::spawn_blocking(move || {
            let state=app.state::<AppState>();
            let _transition=state.managed_agent_runtime_transition.lock().map_err(|error|error.to_string())?;
            let _store=state.managed_agents_store_lock.lock().map_err(|error|error.to_string())?;
            scope(&state,&expected_relay,&expected_owner)?;
            if state.shutdown_started.load(std::sync::atomic::Ordering::Acquire) { return Err("desktop shutdown has started".into()); }
            if let Err(error)=require_idle_runtimes(&app,&state,&relay) { return Ok(outcome.block("writer_ownership_conflict",error)); }
            if !managed_agents::storage::load_project_agent_metadata(&app,&owner)?.iter().any(|agent|agent.pubkey==coordinator_pubkey && agent.local) { return Ok(outcome.block("coordinator_not_managed_locally","Coordinator ownership or local deployment changed; no identity was adopted or replaced.")); }
            let records=managed_agents::load_managed_agents(&app)?;
            let current=records.iter().find(|current|current.pubkey==coordinator_pubkey).ok_or("managed coordinator disappeared")?;
            if current.updated_at!=record.updated_at { return Err("coordinator configuration changed; reload before starting".into()); }
            let mut process=match managed_agents::spawn_agent_child_with_project_transport(&app,current,&relay,true,Some(&owner),None,bridge.as_ref()) { Ok(process)=>process,Err(error)=>return Ok(outcome.block("runtime_start_failed",error)) };
            let key=managed_agents::ManagedAgentRuntimeKey::new(coordinator_pubkey,&relay)?;
            let pid=process.child.id();
            let started_at=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|error|error.to_string())?.as_secs();
            let receipt=managed_agents::ManagedAgentRuntimeReceipt {key:key.clone(),pid,desktop_instance_id:managed_agents::current_instance_id(&app),started_at:crate::util::now_iso()};
            if let Err(error)=managed_agents::write_agent_runtime_receipt(&app,&receipt) {
                managed_agents::terminate_process(pid)?;
                process.child.wait().map_err(|error|error.to_string())?;
                return Err(error);
            }
            state.managed_agent_processes.lock().map_err(|error|error.to_string())?.insert(key,managed_agents::ManagedAgentPairRuntime::starting(process));
            outcome.status="started".into();
            outcome.runtime_receipt=Some(ProjectStartReceipt {pid,started_at});
            Ok(outcome)
        }).await.map_err(|error|error.to_string())?
    }).await.map_err(|error|error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_start_authority_fences_binding_team_identity_channel_and_pause() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let conn = crate::archive::store::open_archive_db(&root.join("archive.db")).unwrap();
        let channel = Uuid::new_v4();
        channel_workspaces::set(&conn, "owner", "relay", channel, 0, root.to_str()).unwrap();
        let snapshot = serde_json::json!({"generation":1,"binding_generation":1,"coordinator_pubkey":"agent","paused":false,"members":[{"pubkey":"agent","role":"coordinator","capabilities":["read"]}]});
        conn.execute(
            "INSERT INTO channel_project_teams VALUES(?1,?2,?3,?4)",
            rusqlite::params!["owner", "relay", channel.to_string(), snapshot.to_string()],
        )
        .unwrap();
        assert!(authority(&conn, "owner", "relay", channel, 1, 1, "agent").is_ok());
        for (owner, relay, channel, binding, team, agent) in [
            ("foreign", "relay", channel, 1, 1, "agent"),
            ("owner", "foreign", channel, 1, 1, "agent"),
            ("owner", "relay", Uuid::new_v4(), 1, 1, "agent"),
            ("owner", "relay", channel, 2, 1, "agent"),
            ("owner", "relay", channel, 1, 2, "agent"),
            ("owner", "relay", channel, 1, 1, "foreign"),
        ] {
            assert!(authority(&conn, owner, relay, channel, binding, team, agent).is_err());
        }
        for patch in [
            serde_json::json!({"paused":true}),
            serde_json::json!({"members":[]}),
            serde_json::json!({"binding_generation":2}),
        ] {
            let mut modified = snapshot.clone();
            for (key, value) in patch.as_object().unwrap() {
                modified[key] = value.clone();
            }
            conn.execute(
                "UPDATE channel_project_teams SET snapshot=?1",
                [modified.to_string()],
            )
            .unwrap();
            assert!(authority(&conn, "owner", "relay", channel, 1, 1, "agent").is_err());
        }
    }
    #[test]
    fn project_start_never_replaces_constrained_or_remote_launchers() {
        assert_eq!(
            transport("codex-acp", &[], "buzz-acp", true),
            if cfg!(target_os = "macos") {
                "codex_reasoning_broker_v1"
            } else {
                "unsupported"
            }
        );
        for (command, args, harness, local) in [
            ("/custom/codex-acp", vec![], "buzz-acp", true),
            ("codex-acp", vec!["restricted".into()], "buzz-acp", true),
            ("codex-acp", vec![], "/custom/buzz-acp", true),
            ("codex-acp", vec![], "buzz-acp", false),
        ] {
            assert_eq!(transport(command, &args, harness, local), "unsupported");
        }
    }
}
