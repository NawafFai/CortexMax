//! Explicit reasoning-only Codex transport; project effects remain in the native broker.

use crate::{mcp::McpRegistry, types::ContentBlock, wire};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::watch,
};

/// Reserved host-owned configuration, never inherited by a model tool.
pub const ENV: &str = "BUZZ_CODEX_PROJECT_BRIDGE";

/// Device-local launch descriptor for an explicitly selected Codex bridge.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeConfig {
    /// Exact owner-selected Codex executable, not the ACP adapter.
    pub binary: String,
    /// Dedicated profile authenticated separately by the owner; never a copied user home.
    pub home: String,
    /// Existing persona's configured model, without fallback substitution.
    pub model: String,
    /// Localhost-only synthetic service for explicit integration tests; desktop never sets it.
    pub synthetic_service: Option<String>,
}

impl BridgeConfig {
    /// Decode the host's explicit opt-in without consulting Codex's ambient configuration.
    pub fn from_env() -> Result<Option<Self>, String> {
        match std::env::var(ENV) {
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err("Codex bridge descriptor is not Unicode".into()),
            Ok(raw) if raw.len() <= 16384 => serde_json::from_str(&raw)
                .map(Some)
                .map_err(|_| "invalid Codex bridge descriptor".into()),
            Ok(_) => Err("Codex bridge descriptor exceeds limit".into()),
        }
    }

    /// Validate a dedicated profile without reading credentials or private provider history.
    pub fn validate(&self) -> Result<(), String> {
        if !cfg!(target_os = "macos") || self.model.is_empty() || self.model.len() > 256 {
            return Err("Codex project bridge requires macOS and an explicit model".into());
        }
        for location in [&self.binary, &self.home] {
            let path = Path::new(location);
            if !path.is_absolute()
                || std::fs::canonicalize(path).map_err(|_| "Codex bridge path unavailable")? != path
            {
                return Err("Codex bridge paths must be canonical, existing, and absolute".into());
            }
        }
        for global in [
            "/etc/codex/config.toml",
            "/etc/codex/requirements.toml",
            "/Library/Preferences/com.openai.codex.plist",
        ] {
            if Path::new(global).exists() {
                return Err(
                    "system Codex configuration requires independent confinement verification"
                        .into(),
                );
            }
        }
        if Path::new(&self.home).join("config.toml").exists() {
            return Err(
                "scoped Codex home must not contain user configuration; do not copy a real profile"
                    .into(),
            );
        }
        if let Some(endpoint) = &self.synthetic_service {
            let url = url::Url::parse(endpoint).map_err(|_| "invalid synthetic service URL")?;
            if url.scheme() != "http"
                || url.host_str() != Some("127.0.0.1")
                || url.port().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(
                    "synthetic model service must be explicit localhost HTTP without credentials"
                        .into(),
                );
            }
        }
        Ok(())
    }

    fn overrides(&self) -> Value {
        let mut config = json!({
            "cli_auth_credentials_store":"file", "check_for_update_on_startup":false,
            "project_doc_max_bytes":0, "sandbox_mode":"read-only", "approval_policy":"on-request",
            "approvals_reviewer":"user", "web_search":"disabled", "mcp_servers":{},
            "analytics":{"enabled":false}, "feedback":{"enabled":false},
            "features": {"shell_tool":false,"unified_exec":false,"plugins":false,"remote_plugin":false,
                "apps":false,"hooks":false,"multi_agent":false,"browser_use":false,"browser_use_external":false,
                "browser_use_full_cdp_access":false,"computer_use":false,"in_app_browser":false,
                "image_generation":false,"view_image":false,"memories":false,"shell_snapshot":false,
                "skill_search":false,"skill_mcp_dependency_install":false,"code_mode":false,"code_mode_host":false}
        });
        if let Some(endpoint) = &self.synthetic_service {
            config["model_provider"] = json!("buzz_fixture");
            config["model_providers"] = json!({"buzz_fixture":{"name":"Buzz synthetic fixture","base_url":endpoint,"wire_api":"responses","requires_openai_auth":false}});
        }
        config
    }
}

pub(crate) struct CodexProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    thread: String,
    next_id: u64,
    model: String,
    _neutral: tempfile::TempDir,
}

impl Drop for CodexProcess {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let _ = self.child.start_kill();
    }
}

impl CodexProcess {
    async fn connect(config: &BridgeConfig) -> Result<Self, String> {
        config.validate()?;
        let neutral = tempfile::Builder::new()
            .prefix("workspace-")
            .tempdir_in(&config.home)
            .map_err(|error| error.to_string())?;
        let mut command = tokio::process::Command::new(&config.binary);
        command
            .arg("app-server")
            .arg("--strict-config")
            .env_clear()
            .env("HOME", &config.home)
            .env("CODEX_HOME", &config.home)
            .env("PATH", "/usr/bin:/bin")
            .current_dir(neutral.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        if config.synthetic_service.is_some() {
            command
                .env("HTTP_PROXY", "http://127.0.0.1:9")
                .env("HTTPS_PROXY", "http://127.0.0.1:9")
                .env("ALL_PROXY", "http://127.0.0.1:9")
                .env("NO_PROXY", "127.0.0.1,localhost");
        }
        let overrides = config.overrides();
        for (key, value) in overrides
            .as_object()
            .ok_or("invalid bridge configuration")?
        {
            let toml_value: toml::Value = serde_json::from_value(value.clone())
                .map_err(|_| "invalid bridge configuration value")?;
            command.arg("-c").arg(format!("{key}={toml_value}"));
        }
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let stdin = child.stdin.take().ok_or("Codex stdin missing")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("Codex stdout missing")?);
        let mut process = Self {
            child,
            stdin,
            stdout,
            thread: String::new(),
            next_id: 0,
            model: config.model.clone(),
            _neutral: neutral,
        };
        let initialized = process.request("initialize", json!({"clientInfo":{"name":"buzz-project-broker","version":"1"},"capabilities":{"experimentalApi":true}})).await?;
        let agent = initialized["userAgent"]
            .as_str()
            .ok_or("Codex version acknowledgement missing")?;
        if !agent.starts_with("buzz-project-broker/0.153.4 ") {
            return Err("this Codex binary has not passed the reasoning-only project contract; configured runtime was not replaced".into());
        }
        process.send(json!({"method":"initialized"})).await?;
        Ok(process)
    }

    pub(crate) async fn start(
        config: &BridgeConfig,
        tools: &McpRegistry,
        instructions: &str,
    ) -> Result<Self, String> {
        tools.validate_project_session()?;
        let mut process = Self::connect(config).await?;
        if config.synthetic_service.is_none() && !process.authenticated().await? {
            return Err("scoped Codex profile requires separate owner authentication; ambient accounts are not used".into());
        }
        let dynamic: Vec<_> = tools.tools().into_iter().map(|tool|json!({"type":"function","name":tool.name,"description":tool.description,"inputSchema":tool.input_schema})).collect();
        let result = process.request("thread/start", json!({"cwd":process._neutral.path(),"model":config.model,"sandbox":"read-only","approvalPolicy":"on-request","approvalsReviewer":"user","ephemeral":true,"dynamicTools":dynamic,"baseInstructions":instructions,"config":config.overrides()})).await?;
        if result["model"] != config.model || result["sandbox"]["type"] != "readOnly" {
            return Err(
                "Codex did not acknowledge the selected model and read-only fallback policy".into(),
            );
        }
        process.thread = result["thread"]["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Codex thread acknowledgement missing")?
            .into();
        Ok(process)
    }

    async fn authenticated(&mut self) -> Result<bool, String> {
        let response = self
            .request("account/read", json!({"refreshToken":false}))
            .await?;
        if !response["requiresOpenaiAuth"].is_boolean() {
            return Err("Codex authentication acknowledgement missing".into());
        }
        Ok(!response["account"].is_null())
    }

    pub(crate) async fn stop(&mut self) -> Result<(), String> {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            match nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            ) {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                Err(error) => return Err(format!("Codex process containment failed: {error}")),
            }
        }
        tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .map_err(|_| "Codex shutdown timed out")?
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn send(&mut self, value: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("Codex request exceeds limit".into());
        }
        bytes.push(b'\n');
        tokio::time::timeout(Duration::from_secs(5), self.stdin.write_all(&bytes))
            .await
            .map_err(|_| "Codex write timed out")?
            .map_err(|error| error.to_string())
    }

    async fn receive(&mut self) -> Result<Value, String> {
        let line = tokio::time::timeout(
            Duration::from_secs(30),
            wire::read_bounded_line(&mut self.stdout, 1024 * 1024),
        )
        .await
        .map_err(|_| "Codex response timed out")?
        .map_err(|error| error.to_string())?
        .ok_or("Codex process closed its output")?;
        serde_json::from_str(&line).map_err(|_| "invalid Codex frame".into())
    }

    async fn refuse(&mut self, request: &Value) -> Result<(), String> {
        self.send(json!({"id":request["id"],"error":{"code":-32601,"message":"Buzz project broker does not authorize this request"}})).await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id":id,"method":method,"params":params}))
            .await?;
        for _frame in 0..128 {
            let message = self.receive().await?;
            if message.get("method").is_some() && message.get("id").is_some() {
                self.refuse(&message).await?;
            } else if message["id"] == id {
                return if message.get("error").is_some() {
                    Err("Codex rejected the scoped request; check scoped authentication and model availability".into())
                } else {
                    message
                        .get("result")
                        .cloned()
                        .ok_or("Codex response has no result".into())
                };
            }
        }
        Err("Codex acknowledgement frame limit exceeded".into())
    }

    pub(crate) async fn prompt(
        &mut self,
        model: &str,
        input: Vec<ContentBlock>,
        tools: Arc<McpRegistry>,
        cancel: &mut watch::Receiver<bool>,
        session: &str,
        wire_tx: &wire::WireSender,
    ) -> Result<(), String> {
        if model != self.model {
            return Err("Codex bridge model changed; create a new scoped session".into());
        }
        if *cancel.borrow() {
            return Err("Codex turn cancelled before dispatch".into());
        }
        tools.validate_project_session()?;
        let input: Vec<_> = input
            .into_iter()
            .map(|block| match block {
                ContentBlock::Text { text } => Ok(json!({"type":"text","text":text})),
                _ => Err("Codex project bridge accepts text only"),
            })
            .collect::<Result<_, _>>()?;
        let input_bytes: usize = input
            .iter()
            .filter_map(|block| block["text"].as_str())
            .map(str::len)
            .sum();
        if input_bytes > 128 * 1024
            || input.iter().all(|block| {
                block["text"]
                    .as_str()
                    .is_none_or(|text| text.trim().is_empty())
            })
        {
            return Err("Codex project task must contain nonempty text within 128 KiB".into());
        }
        let started = tokio::select! {
            _ = cancel.changed() => return Err("Codex turn cancelled during dispatch; no automatic retry".into()),
            result = tokio::time::timeout(Duration::from_secs(30), self.request("turn/start",json!({"threadId":self.thread,"model":self.model,"approvalPolicy":"on-request","approvalsReviewer":"user","sandboxPolicy":{"type":"readOnly","networkAccess":false},"input":input}))) => result.map_err(|_| "Codex turn acknowledgement timed out; no automatic retry")??,
        };
        let turn = started["turn"]["id"]
            .as_str()
            .ok_or("Codex turn acknowledgement missing")?
            .to_string();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        let mut calls = 0;
        for _frame in 0..4096 {
            let message = tokio::select! {
                _ = cancel.changed() => return Err("Codex project turn cancelled; reconcile any pending edit before retry".into()),
                _ = tokio::time::sleep_until(deadline) => return Err("Codex project turn exceeded bounded deadline".into()),
                message = self.receive() => message?,
            };
            tools.validate_project_session()?;
            let method = message["method"].as_str().unwrap_or("");
            let params = &message["params"];
            if message.get("id").is_some() && !method.is_empty() {
                if method != "item/tool/call" {
                    self.refuse(&message).await?;
                    continue;
                }
                calls += 1;
                if calls > 64
                    || params["threadId"] != self.thread
                    || params["turnId"] != turn
                    || !params["namespace"].is_null()
                {
                    self.refuse(&message).await?;
                    return Err(
                        "Codex tool request does not match this bounded project turn".into(),
                    );
                }
                let name = params["tool"].as_str().ok_or("missing dynamic tool name")?;
                let call_id = params["callId"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                    .ok_or("invalid dynamic tool call id")?;
                if !tools.tools().iter().any(|tool| tool.name == name) {
                    self.refuse(&message).await?;
                    return Err("Codex requested an unauthorized project tool".into());
                }
                let result = tools
                    .call(
                        name,
                        &format!("{turn}:{call_id}"),
                        &params["arguments"],
                        crate::mcp::ResultBudget {
                            total: 128 * 1024,
                            text: 128 * 1024,
                        },
                        cancel,
                    )
                    .await;
                let (text, success) = match result {
                    Ok(result) => (
                        result
                            .content
                            .into_iter()
                            .filter_map(|item| match item {
                                crate::types::ToolResultContent::Text(text) => Some(text),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        !result.is_error,
                    ),
                    Err(error) => (error.to_string(), false),
                };
                self.send(json!({"id":message["id"],"result":{"contentItems":[{"type":"inputText","text":text}],"success":success}})).await?;
            } else if method == "item/agentMessage/delta" {
                if params["threadId"] != self.thread || params["turnId"] != turn {
                    return Err("cross-session Codex output refused".into());
                }
                if let Some(text) = params["delta"].as_str() {
                    wire::send_checked(wire_tx,wire::session_update(session,json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}))).await.map_err(|_|"ACP output closed")?;
                }
            } else if method == "turn/completed" {
                if params["threadId"] != self.thread
                    || params["turn"]["id"] != turn
                    || params["turn"]["status"] != "completed"
                {
                    return Err("Codex turn did not complete successfully in this session".into());
                }
                return Ok(());
            } else if method == "error" {
                return Err(
                    "Codex reported a turn failure; no automatic retry was dispatched".into(),
                );
            }
        }
        Err("Codex event limit exceeded".into())
    }
}

/// Check only the dedicated account via Codex's own protocol, without starting a model turn.
/// This proves scoped account presence, not provider access or model readiness.
pub async fn probe_scoped_authentication(config: &BridgeConfig) -> Result<bool, String> {
    tokio::time::timeout(Duration::from_secs(35), async {
        let mut process = CodexProcess::connect(config).await?;
        let result = process.authenticated().await;
        process.stop().await?;
        result
    })
    .await
    .map_err(|_| "scoped Codex authentication probe timed out")?
}
