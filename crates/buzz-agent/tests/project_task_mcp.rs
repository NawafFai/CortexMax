//! The task-board MCP server (`buzz-agent --project-task-mcp`) driven over its real
//! stdio transport against a real SQLite authority store: the same binary and the
//! same tables a harness-gated Claude/Codex session would use.

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

const CHANNEL: &str = "12345678-1234-4234-8234-123456789012";

struct Server {
    child: std::process::Child,
    reader: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl Server {
    fn spawn(policy: &Value) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-agent"))
            .arg("--project-task-mcp")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("BUZZ_NATIVE_PROJECT_POLICY", policy.to_string())
            .env("BUZZ_PROJECT_TASK_CHANNEL", CHANNEL)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn task MCP server");
        let reader = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            reader,
            next_id: 0,
        }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .expect("write request");
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let mut line = String::new();
            let read = self.reader.read_line(&mut line).expect("read response");
            assert!(read > 0, "server closed stdout before answering {method}");
            let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        response["result"].clone()
    }

    fn tool_names(&mut self) -> Vec<String> {
        let response = self.request("tools/list", json!({}));
        let mut names: Vec<String> = response["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn fixture() -> (tempfile::TempDir, Value, std::path::PathBuf) {
    let directory = tempfile::Builder::new()
        .prefix("buzz-task-mcp-")
        .tempdir()
        .unwrap();
    let base = directory.path().canonicalize().unwrap();
    let root = base.join("project");
    std::fs::create_dir(&root).unwrap();
    let database = base.join("archive.db");
    std::fs::write(base.join("managed-agents.json"), "[]").unwrap();
    let conn = Connection::open(&database).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;
        CREATE TABLE channel_workspaces(identity_pubkey TEXT,relay_url TEXT,channel_id TEXT,generation INTEGER,root TEXT);
        CREATE TABLE channel_project_teams(identity_pubkey TEXT,relay_url TEXT,channel_id TEXT,snapshot TEXT,PRIMARY KEY(identity_pubkey,relay_url,channel_id));
        CREATE TABLE channel_project_team_revisions(identity_pubkey TEXT,relay_url TEXT,channel_id TEXT,generation INTEGER,actor TEXT,snapshot TEXT,changed_at INTEGER,PRIMARY KEY(identity_pubkey,relay_url,channel_id,generation));
        CREATE TABLE archive_migrations(name TEXT PRIMARY KEY,applied_at INTEGER);").unwrap();
    buzz_agent::project_workspace::edits::migrate(&conn).unwrap();
    buzz_agent::project_workspace::approvals::migrate(&conn).unwrap();
    buzz_agent::project_workspace::tasks::migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO channel_workspaces VALUES('owner','wss://example.test',?1,1,?2)",
        params![CHANNEL, root.to_str()],
    )
    .unwrap();
    let team = json!({"generation":1,"binding_generation":1,"paused":false,"coordinator_pubkey":"coordinator",
        "members":[{"pubkey":"coordinator","role":"coordinator","capabilities":["read","assign_tasks"]},
                   {"pubkey":"agent","role":"writer","capabilities":["read","edit"]}]});
    conn.execute(
        "INSERT INTO channel_project_teams VALUES('owner','wss://example.test',?1,?2)",
        params![CHANNEL, team.to_string()],
    )
    .unwrap();
    let policy = json!({
        "database": database.to_str().unwrap(),
        "owner_pubkey": "owner",
        "relay_url": "wss://example.test",
        "agent_pubkey": "agent",
        "protected_roots": [],
        "agent_store": base.join("managed-agents.json").to_str().unwrap(),
    });
    (directory, policy, database)
}

#[test]
fn task_mcp_server_serves_grant_scoped_tools_over_real_stdio() {
    let (_directory, policy, database) = fixture();
    let mut server = Server::spawn(&policy);
    let init = server.request(
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}),
    );
    assert_eq!(
        init["result"]["serverInfo"]["name"], "buzz-project-tasks",
        "{init}"
    );
    server.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));

    // A writer sees the base tools only: no create/assign/review/report.
    let names = server.tool_names();
    assert_eq!(
        names,
        vec![
            "project__task_block",
            "project__task_deliver",
            "project__task_get",
            "project__task_list",
            "project__task_request_input",
            "project__task_start",
        ]
    );
    let listed = server.call("project__task_list", json!({}));
    assert_eq!(listed["isError"], json!(false), "{listed}");
    let text: Value = serde_json::from_str(listed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["tasks"], json!([]));

    // A grant the agent lacks is refused as a tool error, never executed.
    let refused = server.call(
        "project__task_create",
        json!({"title":"x","description":"y"}),
    );
    assert_eq!(refused["isError"], json!(true), "{refused}");

    // The owner plans and assigns a task through the shared state machine; the
    // agent then advances it over MCP with the correct revision only.
    let conn = Connection::open(&database).unwrap();
    let members = vec![
        (
            "coordinator".to_string(),
            vec!["read".to_string(), "assign_tasks".to_string()],
        ),
        (
            "agent".to_string(),
            vec!["read".to_string(), "edit".to_string()],
        ),
    ];
    let team = buzz_agent::project_workspace::tasks::TeamView {
        coordinator: Some("coordinator"),
        members: &members,
        paused: false,
        binding_generation: 1,
        team_generation: 1,
    };
    let channel = uuid::Uuid::parse_str(CHANNEL).unwrap();
    let task = buzz_agent::project_workspace::tasks::create(
        &conn,
        "owner",
        "wss://example.test",
        channel,
        buzz_agent::project_workspace::tasks::Actor::Owner,
        &team,
        buzz_agent::project_workspace::tasks::NewTask {
            title: "Write tests",
            description: "اكتب اختبارات للمحلل",
            required_inputs: &[],
            dependencies: &[],
            return_to: "owner",
        },
    )
    .unwrap();
    let task = buzz_agent::project_workspace::tasks::transition(
        &conn,
        "owner",
        "wss://example.test",
        &task.task_id,
        task.revision,
        buzz_agent::project_workspace::tasks::Actor::Owner,
        &team,
        buzz_agent::project_workspace::tasks::Transition::Assign { assignee: "agent" },
    )
    .unwrap();
    drop(conn);

    let stale = server.call(
        "project__task_start",
        json!({"task_id": task.task_id, "expected_revision": task.revision - 1}),
    );
    assert_eq!(
        stale["isError"],
        json!(true),
        "stale revision must be refused: {stale}"
    );
    let started = server.call(
        "project__task_start",
        json!({"task_id": task.task_id, "expected_revision": task.revision}),
    );
    assert_eq!(started["isError"], json!(false), "{started}");
    let started: Value =
        serde_json::from_str(started["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(started["state"], "running");
    let delivered = server.call(
        "project__task_deliver",
        json!({"task_id": task.task_id, "expected_revision": started["revision"],
               "outputs": [{"kind":"file","reference":"tests/parser.rs"}]}),
    );
    let delivered: Value =
        serde_json::from_str(delivered["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(delivered["state"], "delivered");
    // Delivery is recorded once.
    let again = server.call(
        "project__task_deliver",
        json!({"task_id": task.task_id, "expected_revision": delivered["revision"],
               "outputs": [{"kind":"note","reference":"again"}]}),
    );
    assert_eq!(again["isError"], json!(true));

    // The desktop sees the delivery and the dispatch intent for the owner.
    let conn = Connection::open(&database).unwrap();
    let stored = buzz_agent::project_workspace::tasks::get(
        &conn,
        "owner",
        "wss://example.test",
        &task.task_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(stored.state, "delivered");
    assert_eq!(stored.dispatch_state, "pending");
    assert_eq!(stored.dispatch_target.as_deref(), Some("owner"));

    // Pausing the team makes the very next call refuse project work.
    conn.execute(
        "UPDATE channel_project_teams SET snapshot = ?1 WHERE channel_id = ?2",
        params![
            json!({"generation":2,"binding_generation":1,"paused":true,"coordinator_pubkey":"coordinator",
                "members":[{"pubkey":"coordinator","role":"coordinator","capabilities":["read","assign_tasks"]},
                           {"pubkey":"agent","role":"writer","capabilities":["read","edit"]}]}).to_string(),
            CHANNEL
        ],
    )
    .unwrap();
    let paused = server.request("tools/list", json!({}));
    assert!(
        paused.get("error").is_some(),
        "a paused team must not advertise task tools: {paused}"
    );
}
