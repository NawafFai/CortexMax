//! Stdio MCP server that exposes the task board to harness-gated runtimes.
//!
//! Claude Code and Codex sessions bound to a channel project cannot call the
//! native `buzz-agent` tools directly, so the Buzz harness registers this server
//! (`buzz-agent --project-task-mcp`) at `session/new`. The server carries the same
//! device-local policy locator the native runtime uses and one fixed channel; it
//! establishes the acting identity from the policy, re-checks authority on every
//! call, and exposes exactly the task tools the agent's current grants allow.
//! It never touches project files, runs commands, or messages the relay.

use super::{task_tools, ProjectPolicy};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
};
use std::sync::Arc;

/// Environment key naming the channel this server serves.
pub const CHANNEL_ENV: &str = "BUZZ_PROJECT_TASK_CHANNEL";
/// Launch argument selecting this mode.
pub const LAUNCH_ARG: &str = "--project-task-mcp";

/// One server instance: policy locator plus the channel it serves.
#[derive(Clone)]
pub struct TaskServer {
    policy: ProjectPolicy,
    channel: String,
}

impl TaskServer {
    /// Build from the process environment written by the Buzz harness.
    pub fn from_env() -> Result<Self, String> {
        let policy = ProjectPolicy::from_env()?
            .ok_or("task MCP server requires the native project policy in its environment")?;
        let channel = std::env::var(CHANNEL_ENV)
            .map_err(|_| format!("task MCP server requires {CHANNEL_ENV}"))?;
        uuid::Uuid::parse_str(&channel).map_err(|_| "task MCP channel must be a UUID")?;
        Ok(Self { policy, channel })
    }

    fn grants(&self) -> Result<super::SessionGrants, McpError> {
        match self.policy.session_grants(&self.channel) {
            Ok(Some(grants)) => Ok(grants),
            Ok(None) => Err(McpError::invalid_request(
                "this channel has no attached project",
                None,
            )),
            Err(reason) => Err(McpError::invalid_request(reason, None)),
        }
    }
}

fn to_tool(def: task_tools::TaskToolDef) -> Tool {
    let schema = match def.input_schema {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    Tool::new(def.name, def.description, Arc::new(schema))
}

impl ServerHandler for TaskServer {
    fn get_info(&self) -> ServerInfo {
        let mut info =
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
                Implementation::new("buzz-project-tasks", env!("CARGO_PKG_VERSION")),
            );
        info.instructions = Some(
            "Buzz channel project task board. State advances only through these tools; chat text is never a delivery. Read project__task_list before acting, pass the task's current revision, and report blockers or missing inputs through the tools."
                .to_string(),
        );
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let server = self.clone();
        let grants = tokio::task::spawn_blocking(move || server.grants())
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))??;
        Ok(ListToolsResult {
            meta: None,
            next_cursor: None,
            tools: task_tools::definitions(&grants.capabilities)
                .into_iter()
                .map(to_tool)
                .collect(),
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.to_string();
        if !task_tools::is_task_tool(&name) {
            return Err(McpError::invalid_params(
                format!("unknown tool {name}"),
                None,
            ));
        }
        let arguments = serde_json::Value::Object(request.arguments.unwrap_or_default());
        let server = self.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let grants = server.grants()?;
            task_tools::call(&server.policy, &grants.scope, &name, &arguments)
                .map_err(|reason| McpError::invalid_request(reason, None))
        })
        .await
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        match outcome {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(
                error.message.to_string(),
            )])),
        }
    }
}

/// Serve the task board when launched with [`LAUNCH_ARG`]; `None` otherwise.
///
/// Runs on its own current-thread runtime and returns only when the client
/// closes the transport.
pub fn serve_if_requested() -> Option<Result<(), String>> {
    if std::env::args().nth(1).as_deref() != Some(LAUNCH_ARG) {
        return None;
    }
    Some(serve())
}

fn serve() -> Result<(), String> {
    let server = TaskServer::from_env()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| error.to_string())?;
        service.waiting().await.map_err(|error| error.to_string())?;
        Ok(())
    })
}
