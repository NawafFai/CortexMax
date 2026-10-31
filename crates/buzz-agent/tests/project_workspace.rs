#![cfg(target_os = "macos")]

use buzz_agent::project_workspace::ProjectPolicy;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
mod common;
use common::{spawn_capturing_llm, Harness};

struct Fixture {
    _directory: tempfile::TempDir,
    root: std::path::PathBuf,
    database: std::path::PathBuf,
    policy: ProjectPolicy,
    scope: buzz_core::local_workspace::ProjectSessionScope,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("buzz-project-")
            .tempdir()
            .unwrap();
        let base = directory.path().canonicalize().unwrap();
        let root = base.join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("sample.txt"), "synthetic project content").unwrap();
        std::fs::write(base.join("outside.txt"), "synthetic outside sentinel").unwrap();
        std::fs::write(root.join("AGENTS.md"), "HINT_MUST_NOT_LOAD").unwrap();
        std::os::unix::fs::symlink(base.join("outside.txt"), root.join("escape.txt")).unwrap();
        std::fs::hard_link(base.join("outside.txt"), root.join("hard.txt")).unwrap();
        let database = base.join("archive.db");
        std::fs::write(base.join("managed-agents.json"), "[]").unwrap();
        let conn = Connection::open(&database).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE channel_workspaces(identity_pubkey TEXT,relay_url TEXT,channel_id TEXT,generation INTEGER,root TEXT);
            CREATE TABLE channel_project_teams(identity_pubkey TEXT,relay_url TEXT,channel_id TEXT,snapshot TEXT,PRIMARY KEY(identity_pubkey,relay_url,channel_id));
            CREATE TABLE channel_project_team_revisions(identity_pubkey TEXT,relay_url TEXT,channel_id TEXT,generation INTEGER,actor TEXT,snapshot TEXT,changed_at INTEGER,PRIMARY KEY(identity_pubkey,relay_url,channel_id,generation));
            CREATE TABLE archive_migrations(name TEXT PRIMARY KEY,applied_at INTEGER);").unwrap();
        buzz_agent::project_workspace::edits::migrate(&conn).unwrap();
        let channel = "12345678-1234-4234-8234-123456789012";
        conn.execute(
            "INSERT INTO channel_workspaces VALUES('owner','wss://example.test',?1,1,?2)",
            params![channel, root.to_str()],
        )
        .unwrap();
        let team = json!({"generation":1,"binding_generation":1,"paused":false,"coordinator_pubkey":"agent","members":[{"pubkey":"agent","role":"coordinator","capabilities":["read"]}]});
        conn.execute(
            "INSERT INTO channel_project_teams VALUES('owner','wss://example.test',?1,?2)",
            params![channel, team.to_string()],
        )
        .unwrap();
        let policy = ProjectPolicy {
            database: database.to_str().unwrap().into(),
            owner_pubkey: "owner".into(),
            relay_url: "wss://example.test".into(),
            agent_pubkey: "agent".into(),
            protected_roots: vec![],
            agent_store: base.join("managed-agents.json").to_str().unwrap().into(),
        };
        let scope = policy.session_scope(channel).unwrap().unwrap();
        Self {
            _directory: directory,
            root,
            database,
            policy,
            scope,
        }
    }

    async fn harness(&self, url: &str) -> Harness {
        Harness::spawn_with_env(
            url,
            &[
                (
                    "BUZZ_NATIVE_PROJECT_POLICY",
                    &serde_json::to_string(&self.policy).unwrap(),
                ),
                ("BUZZ_AGENT_HINTS", "true"),
            ],
        )
        .await
    }
}

fn response(calls: Vec<Value>) -> Value {
    json!({"id":"synthetic","object":"chat.completion","model":"fake-model","choices":[{"index":0,"message":{"role":"assistant","content":"done","tool_calls":calls},"finish_reason":if calls.is_empty(){"stop"}else{"tool_calls"}}]})
}

fn read_call(name: &str, path: &str) -> Value {
    json!({"id":name,"type":"function","function":{"name":"project__read_text","arguments":json!({"path":path}).to_string()}})
}

#[tokio::test]
#[ignore = "requires explicit BUZZ_TEST_CODEX 0.153.4 binary and authorized localhost fixture"]
async fn codex_project_actual_broker_rejects_builtins_and_confines_native_tools() {
    use axum::{extract::State, routing::post, Json, Router};
    use std::sync::Arc;
    use tokio::sync::Mutex;
    async fn reply(
        State(captured): State<Arc<Mutex<Vec<Value>>>>,
        Json(request): Json<Value>,
    ) -> impl axum::response::IntoResponse {
        let mut captured = captured.lock().await;
        captured.push(request);
        let index = captured.len();
        let calls = [
            ("project__read_text", json!({"path":"sample.txt"})),
            ("project__read_text", json!({"path":"../outside.txt"})),
            ("project__read_text", json!({"path":"escape.txt"})),
            (
                "project__create_text",
                json!({"path":"created.txt","content":"broker-created"}),
            ),
            ("exec_command", json!({"cmd":"/bin/touch forbidden.txt"})),
            ("view_image", json!({"path":"/etc/hosts"})),
            ("mcp__synthetic__write", json!({"path":"forbidden.txt"})),
        ];
        let item = if let Some((name, arguments)) = calls.get(index - 1) {
            json!({"id":format!("fc-{index}"),"type":"function_call","call_id":format!("call-{index}"),"name":name,"arguments":arguments.to_string()})
        } else if index == 8 {
            json!({"id":"patch-fixture","type":"custom_tool_call","call_id":"patch-call","name":"apply_patch","input":"*** Begin Patch\n*** Add File: forbidden.txt\n+bad\n*** End Patch"})
        } else {
            json!({"id":"msg-fixture","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"synthetic complete","annotations":[]}]})
        };
        let events = [
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
            json!({"type":"response.completed","response":{"id":format!("resp-{index}"),"status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
        ];
        let body = events
            .iter()
            .map(|event| {
                format!(
                    "event: {}\ndata: {event}\n\n",
                    event["type"].as_str().unwrap()
                )
            })
            .collect::<String>();
        ([("content-type", "text/event-stream")], body)
    }
    let binary = std::env::var("BUZZ_TEST_CODEX").expect("explicit test binary required");
    let fixture = Fixture::new();
    let home = fixture.root.parent().unwrap().join("codex-home");
    std::fs::create_dir(&home).unwrap();
    let conn = Connection::open(&fixture.database).unwrap();
    conn.execute("UPDATE channel_project_teams SET snapshot=?1", [json!({"generation":1,"binding_generation":1,"paused":false,"coordinator_pubkey":"agent","members":[{"pubkey":"agent","role":"coordinator","capabilities":["read","edit"]}]}).to_string()]).unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/v1/responses", post(reply))
        .with_state(captured.clone());
    let service = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let descriptor = buzz_agent::codex_project::BridgeConfig {
        binary,
        home: home.to_str().unwrap().into(),
        model: "fixture".into(),
        synthetic_service: Some(endpoint),
    };
    assert!(
        !buzz_agent::codex_project::probe_scoped_authentication(&descriptor)
            .await
            .unwrap()
    );
    assert!(captured.lock().await.is_empty());
    let mut harness = Harness::spawn_with_env(
        "http://127.0.0.1:9",
        &[
            (
                "BUZZ_NATIVE_PROJECT_POLICY",
                &serde_json::to_string(&fixture.policy).unwrap(),
            ),
            (
                buzz_agent::codex_project::ENV,
                &serde_json::to_string(&descriptor).unwrap(),
            ),
        ],
    )
    .await;
    initialize(&mut harness).await;
    let id = harness
        .send(
            "session/new",
            json!({"cwd":fixture.root,"projectSession":fixture.scope,"mcpServers":[]}),
        )
        .await;
    let session = harness.recv_until(|message| message["id"] == id).await;
    assert!(
        session.get("result").is_some(),
        "{session}; {}",
        harness.stderr_text()
    );
    assert_eq!(
        session["result"]["reasoningTransport"],
        "codex_reasoning_broker_v1"
    );
    let id = harness.send("session/prompt",json!({"sessionId":session["result"]["sessionId"],"prompt":[{"type":"text","text":"Exercise synthetic broker tools."}]})).await;
    let result = harness.recv_until(|message| message["id"] == id).await;
    assert!(
        result.get("result").is_some(),
        "{result}; {}",
        harness.stderr_text()
    );
    let requests = captured.lock().await;
    assert_eq!(requests.len(), 9, "{requests:?}");
    let inventory = requests[0]["tools"].as_array().unwrap();
    let names: Vec<_> = inventory
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(
        names.iter().all(|name| [
            "project__read_text",
            "project__create_text",
            "project__replace_text",
            "project__get_team",
            "request_user_input"
        ]
        .contains(name)),
        "{names:?}"
    );
    assert!(names.contains(&"project__read_text"));
    let transcript = requests.last().unwrap().to_string();
    assert!(
        transcript.contains("synthetic project content"),
        "{transcript}"
    );
    assert!(!transcript.contains("synthetic outside sentinel"));
    assert!(!transcript.contains("HINT_MUST_NOT_LOAD"));
    for name in ["exec_command", "view_image", "mcp__synthetic__write"] {
        assert!(
            transcript.contains(&format!("unsupported call: {name}")),
            "{transcript}"
        );
    }
    assert!(
        transcript.contains("unsupported custom tool call: apply_patch"),
        "{transcript}"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("created.txt")).unwrap(),
        "broker-created"
    );
    assert!(!fixture.root.join("forbidden.txt").exists());
    assert!(!requests[0]
        .to_string()
        .contains(fixture.root.to_str().unwrap()));
    drop(requests);
    conn.execute("UPDATE channel_workspaces SET generation=2", [])
        .unwrap();
    let id = harness.send("session/prompt",json!({"sessionId":session["result"]["sessionId"],"prompt":[{"type":"text","text":"stale"}]})).await;
    assert!(harness
        .recv_until(|message| message["id"] == id)
        .await
        .get("error")
        .is_some());
    assert_eq!(captured.lock().await.len(), 9);
    harness.shutdown().await;
    service.abort();
}

async fn initialize(harness: &mut Harness) {
    let id = harness
        .send(
            "initialize",
            json!({"protocolVersion":2,"clientCapabilities":{}}),
        )
        .await;
    let result = harness.recv_until(|message| message["id"] == id).await;
    assert!(result.get("result").is_some(), "{result}");
}

#[tokio::test]
async fn native_project_actual_session_reads_confined_file_and_rejects_escape() {
    let fixture = Fixture::new();
    let llm = spawn_capturing_llm(vec![
        response(vec![
            read_call("read", "sample.txt"),
            read_call("traversal", "../outside.txt"),
            read_call("symlink", "escape.txt"),
            read_call("hardlink", "hard.txt"),
        ]),
        response(vec![]),
    ])
    .await;
    let mut harness = fixture.harness(&llm.url).await;
    initialize(&mut harness).await;
    let id = harness
        .send(
            "session/new",
            json!({"cwd":fixture.root,"projectSession":fixture.scope,"mcpServers":[]}),
        )
        .await;
    let session = harness.recv_until(|message| message["id"] == id).await;
    assert_eq!(session["result"]["projectSession"], json!(fixture.scope));
    assert_eq!(
        session["result"]["nativeProjectMode"],
        "native-project-text-v1"
    );
    let id=harness.send("session/prompt",json!({"sessionId":session["result"]["sessionId"],"prompt":[{"type":"text","text":"Read the project."}]})).await;
    let result = harness
        .recv_until_approving(|message| message["id"] == id)
        .await;
    assert!(
        result.get("result").is_some(),
        "{result}; {}",
        harness.stderr_text()
    );
    let requests = llm.captured.lock().await;
    assert_eq!(requests.len(), 2);
    let transcript = requests[1].to_string();
    assert!(
        transcript.contains("synthetic project content"),
        "{transcript}"
    );
    assert!(!transcript.contains("synthetic outside sentinel"));
    assert!(!transcript.contains("HINT_MUST_NOT_LOAD"));
    // A read-only member sees exactly the two native text tools plus the base
    // task-board tools; no edit, create, assign, review or report tool leaks in.
    let mut names: Vec<String> = requests[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            tool["function"]["name"]
                .as_str()
                .or_else(|| tool["name"].as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "project__get_team",
            "project__read_text",
            "project__task_block",
            "project__task_deliver",
            "project__task_get",
            "project__task_list",
            "project__task_request_input",
            "project__task_start",
        ]
    );
    drop(requests);
    harness.shutdown().await;
}

#[tokio::test]
async fn native_project_sessions_reject_mcp_wrong_cwd_stale_grants_and_leave_conversations_toolless(
) {
    let fixture = Fixture::new();
    let llm = spawn_capturing_llm(vec![response(vec![])]).await;
    let mut harness = fixture.harness(&llm.url).await;
    initialize(&mut harness).await;
    for params in [
        json!({"cwd":"/","projectSession":fixture.scope,"mcpServers":[]}),
        json!({"cwd":fixture.root,"projectSession":fixture.scope,"mcpServers":[{"name":"forbidden","command":"/bin/touch","args":["unauthorized"],"env":[]}]}),
    ] {
        let id = harness.send("session/new", params).await;
        let result = harness.recv_until(|message| message["id"] == id).await;
        assert!(result.get("error").is_some(), "{result}");
    }
    let id = harness
        .send(
            "session/new",
            json!({"cwd":fixture.root,"projectSession":fixture.scope,"mcpServers":[]}),
        )
        .await;
    let session = harness.recv_until(|message| message["id"] == id).await;
    let conn = Connection::open(&fixture.database).unwrap();
    conn.execute(
        "UPDATE channel_project_teams SET snapshot=json_set(snapshot,'$.generation',2)",
        [],
    )
    .unwrap();
    let id=harness.send("session/prompt",json!({"sessionId":session["result"]["sessionId"],"prompt":[{"type":"text","text":"Stale work must fail."}]})).await;
    assert!(harness
        .recv_until(|message| message["id"] == id)
        .await
        .get("error")
        .is_some());
    assert!(llm.captured.lock().await.is_empty());
    let id = harness
        .send("session/new", json!({"cwd":fixture.root,"mcpServers":[]}))
        .await;
    let conversation = harness.recv_until(|message| message["id"] == id).await;
    let id=harness.send("session/prompt",json!({"sessionId":conversation["result"]["sessionId"],"prompt":[{"type":"text","text":"Conversation only."}]})).await;
    assert!(harness
        .recv_until_approving(|message| message["id"] == id)
        .await
        .get("result")
        .is_some());
    let requests = llm.captured.lock().await;
    assert!(requests[0]
        .get("tools")
        .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty)));
    assert!(!requests[0].to_string().contains("HINT_MUST_NOT_LOAD"));
    drop(requests);
    assert!(!fixture.root.join("unauthorized").exists());
    harness.shutdown().await;
}

#[test]
fn native_project_authority_serializes_revocation_even_with_wal() {
    let fixture = Fixture::new();
    fixture
        .policy
        .with_authority(&fixture.scope, "read", || {
            let conn = Connection::open(&fixture.database).unwrap();
            conn.busy_timeout(std::time::Duration::ZERO).unwrap();
            assert!(conn
                .execute("UPDATE channel_project_teams SET snapshot='{}'", [])
                .is_err());
            Ok(())
        })
        .unwrap();
    let other = ProjectPolicy {
        agent_pubkey: "other".into(),
        ..fixture.policy.clone()
    };
    assert!(other
        .with_authority(&fixture.scope, "read", || Ok(()))
        .is_err());
    assert!(fixture
        .policy
        .with_authority(&fixture.scope, "edit", || Ok(()))
        .is_err());
}

fn tool_call(id: &str, name: &str, args: Value) -> Value {
    json!({"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}})
}

fn hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(content.as_bytes()))
}

#[tokio::test]
async fn native_project_actual_edits_create_exclusively_preserve_work_and_reconcile_receipts() {
    let fixture = Fixture::new();
    let conn = Connection::open(&fixture.database).unwrap();
    conn.execute("UPDATE channel_project_teams SET snapshot=json_set(snapshot,'$.members[0].capabilities',json('[\"read\",\"edit\"]'))",[]).unwrap();
    let create = tool_call(
        "create1",
        "project__create_text",
        json!({"path":"new.txt","content":"new project file"}),
    );
    let llm=spawn_capturing_llm(vec![
        response(vec![create.clone()]),
        response(vec![tool_call("edit1","project__replace_text",json!({"path":"sample.txt","expected_sha256":hash("synthetic project content"),"content":"bounded change"}))]),
        response(vec![]),
        response(vec![create]),response(vec![]),
        response(vec![tool_call("overwrite","project__create_text",json!({"path":"new.txt","content":"must not overwrite"}))]),response(vec![]),
    ]).await;
    let mut harness = fixture.harness(&llm.url).await;
    initialize(&mut harness).await;
    let id = harness
        .send(
            "session/new",
            json!({"cwd":fixture.root,"projectSession":fixture.scope,"mcpServers":[]}),
        )
        .await;
    let session = harness.recv_until(|message| message["id"] == id).await;
    for turn in 0..3 {
        let id=harness.send("session/prompt",json!({"sessionId":session["result"]["sessionId"],"prompt":[{"type":"text","text":"Bounded synthetic edit."}]})).await;
        let result = harness
            .recv_until_approving(|message| message["id"] == id)
            .await;
        assert!(result.get("result").is_some(), "{result}");
        if turn < 2 {
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM project_file_operations WHERE state='completed'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                2
            );
        }
    }
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("new.txt")).unwrap(),
        "new project file"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("sample.txt")).unwrap(),
        "bounded change"
    );
    let pending =
        buzz_agent::project_workspace::edits::pending(&conn, "owner", "wss://example.test")
            .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].before_sha256, "absent");
    harness.shutdown().await;
    assert!(buzz_agent::project_workspace::edits::reconcile(
        &conn,
        "owner",
        "wss://example.test",
        &pending[0].operation_id,
        &hash("wrong observed state")
    )
    .is_err());
    buzz_agent::project_workspace::edits::reconcile(
        &conn,
        "owner",
        "wss://example.test",
        &pending[0].operation_id,
        &hash("new project file"),
    )
    .unwrap();
    assert!(
        buzz_agent::project_workspace::edits::pending(&conn, "owner", "wss://example.test")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM project_file_operations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        3
    );
}

#[test]
fn native_project_cannot_select_authority_or_identity_roots() {
    let fixture = Fixture::new();
    let conn = Connection::open(&fixture.database).unwrap();
    conn.execute(
        "UPDATE channel_workspaces SET root=?1",
        [fixture.database.parent().unwrap().to_str()],
    )
    .unwrap();
    assert!(fixture
        .policy
        .session_scope(&fixture.scope.channel_id.to_string())
        .is_err());
    conn.execute(
        "UPDATE channel_workspaces SET root=?1",
        [fixture.root.to_str()],
    )
    .unwrap();
    let protected = ProjectPolicy {
        protected_roots: vec![fixture.root.to_str().unwrap().into()],
        ..fixture.policy
    };
    assert!(protected
        .session_scope(&fixture.scope.channel_id.to_string())
        .is_err());
}

#[test]
fn native_project_cannot_bypass_protected_identity_with_symlinked_config_paths() {
    let fixture = Fixture::new();
    let base = fixture.database.parent().unwrap();
    let alias = base.join("identity-link");
    std::os::unix::fs::symlink(&fixture.root, &alias).unwrap();
    let protected = ProjectPolicy {
        protected_roots: vec![alias.to_str().unwrap().into()],
        ..fixture.policy.clone()
    };
    assert!(protected
        .session_scope(&fixture.scope.channel_id.to_string())
        .is_err());
    std::fs::write(fixture.root.join("identity.json"), "[]").unwrap();
    let alias_file = base.join("agent-store-link.json");
    std::os::unix::fs::symlink(fixture.root.join("identity.json"), &alias_file).unwrap();
    let protected = ProjectPolicy {
        agent_store: alias_file.to_str().unwrap().into(),
        ..fixture.policy
    };
    assert!(protected
        .session_scope(&fixture.scope.channel_id.to_string())
        .is_err());
}

#[tokio::test]
async fn native_project_coordinator_delegates_and_releases_existing_staff_without_escalation() {
    let mut fixture = Fixture::new();
    let owner = nostr::Keys::generate();
    let manager = nostr::Keys::generate();
    let staff = nostr::Keys::generate();
    let foreign = nostr::Keys::generate();
    let foreign_owner = nostr::Keys::generate();
    let manager_key = manager.public_key().to_hex();
    let manager_grant =
        json!({"pubkey":manager_key,"role":"coordinator","capabilities":["read","manage_team"]});
    let staff_grant =
        json!({"pubkey":staff.public_key().to_hex(),"role":"member","capabilities":["read"]});
    let mut escalated = staff_grant.clone();
    escalated["capabilities"] = json!(["read", "edit"]);
    let unowned =
        json!({"pubkey":foreign.public_key().to_hex(),"role":"member","capabilities":["read"]});
    let records=vec![(&manager,&owner),(&staff,&owner),(&foreign,&foreign_owner)].into_iter().map(|(agent,attestor)|json!({
        "pubkey":agent.public_key().to_hex(),"name":"synthetic staff","auth_tag":buzz_sdk::nip_oa::compute_auth_tag(attestor,&agent.public_key(),"").unwrap(),
        "private_key_nsec":{"sentinel":"NEVER_EXPORT_METADATA_SECRET"}
    })).collect::<Vec<_>>();
    let metadata = serde_json::to_vec(&records).unwrap();
    std::fs::write(&fixture.policy.agent_store, &metadata).unwrap();
    let conn = Connection::open(&fixture.database).unwrap();
    let owner_key = owner.public_key().to_hex();
    conn.execute(
        "UPDATE channel_workspaces SET identity_pubkey=?1",
        [&owner_key],
    )
    .unwrap();
    conn.execute("UPDATE channel_project_teams SET identity_pubkey=?1,snapshot=?2",params![owner_key,json!({"generation":1,"binding_generation":1,"paused":false,"coordinator_pubkey":manager_key,"members":[manager_grant]}).to_string()]).unwrap();
    fixture.policy.owner_pubkey = owner.public_key().to_hex();
    fixture.policy.agent_pubkey = manager.public_key().to_hex();
    fixture.scope = fixture
        .policy
        .session_scope(&fixture.scope.channel_id.to_string())
        .unwrap()
        .unwrap();
    let llm = spawn_capturing_llm(vec![
        response(vec![tool_call(
            "escalate",
            "project__update_team",
            json!({"members":[manager_grant,escalated]}),
        )]),
        response(vec![tool_call(
            "unowned",
            "project__update_team",
            json!({"members":[manager_grant,unowned]}),
        )]),
        response(vec![tool_call(
            "delegate",
            "project__update_team",
            json!({"members":[manager_grant,staff_grant]}),
        )]),
        response(vec![read_call("stale-read", "sample.txt")]),
        response(vec![]),
        response(vec![tool_call(
            "release",
            "project__update_team",
            json!({"members":[manager_grant]}),
        )]),
        response(vec![]),
    ])
    .await;
    let mut harness = fixture.harness(&llm.url).await;
    initialize(&mut harness).await;
    let id = harness
        .send(
            "session/new",
            json!({"cwd":fixture.root,"projectSession":fixture.scope,"mcpServers":[]}),
        )
        .await;
    let session = harness.recv_until(|message| message["id"] == id).await;
    let id=harness.send("session/prompt",json!({"sessionId":session["result"]["sessionId"],"prompt":[{"type":"text","text":"Bounded team delegation."}]})).await;
    let result = harness
        .recv_until_approving(|message| message["id"] == id)
        .await;
    assert!(result.get("result").is_some(), "{result}");
    let current = buzz_agent::project_workspace::teams::load(
        &conn,
        &fixture.policy.owner_pubkey,
        &fixture.policy.relay_url,
        fixture.scope.channel_id,
    )
    .unwrap();
    assert_eq!(current.generation, 2);
    assert_eq!(current.members.len(), 2);
    assert_eq!(current.members[1].capabilities, vec!["read"]);
    assert!(!llm.captured.lock().await[4]
        .to_string()
        .contains("synthetic project content"));
    let id=harness.send("session/prompt",json!({"sessionId":session["result"]["sessionId"],"prompt":[{"type":"text","text":"stale"}]})).await;
    assert!(harness
        .recv_until(|message| message["id"] == id)
        .await
        .get("error")
        .is_some());
    let id = harness
        .send(
            "session/dispose",
            json!({"sessionId":session["result"]["sessionId"]}),
        )
        .await;
    assert_eq!(
        harness.recv_until(|message| message["id"] == id).await["result"]["disposed"],
        true
    );
    let fresh = fixture
        .policy
        .session_scope(&fixture.scope.channel_id.to_string())
        .unwrap()
        .unwrap();
    let id = harness
        .send(
            "session/new",
            json!({"cwd":fixture.root,"projectSession":fresh,"mcpServers":[]}),
        )
        .await;
    let session = harness.recv_until(|message| message["id"] == id).await;
    let id=harness.send("session/prompt",json!({"sessionId":session["result"]["sessionId"],"prompt":[{"type":"text","text":"Release only project membership."}]})).await;
    assert!(harness
        .recv_until_approving(|message| message["id"] == id)
        .await
        .get("result")
        .is_some());
    let current = buzz_agent::project_workspace::teams::load(
        &conn,
        &fixture.policy.owner_pubkey,
        &fixture.policy.relay_url,
        fixture.scope.channel_id,
    )
    .unwrap();
    assert_eq!(current.members.len(), 1);
    assert_eq!(current.generation, 3);
    assert_eq!(
        std::fs::read(&fixture.policy.agent_store).unwrap(),
        metadata
    );
    assert!(!llm
        .captured
        .lock()
        .await
        .iter()
        .any(|request| request.to_string().contains("NEVER_EXPORT_METADATA_SECRET")));
    harness.shutdown().await;
}
