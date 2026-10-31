//! Tell the owner when an agent drives or reads their screen.
//!
//! Screen-control tools reach agents from the owner's own Claude Code and
//! Codex settings — KimiCU, computer-use — so any agent on this Mac can click,
//! type and take screenshots with nothing on screen to say so. The harness
//! sees every tool call an agent starts; each one that touches the screen is
//! written to the file the desktop names in [`COMPUTER_USE_FILE_ENV`], and the
//! desktop floats the agent's name above every window with a Stop button.
//! Looking counts as much as clicking: a screenshot shows whatever is open.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Where the desktop wants each screen-control call reported.
pub const COMPUTER_USE_FILE_ENV: &str = "BUZZ_ACP_COMPUTER_USE_FILE";

/// Tool servers that act on the screen, as tool names spell them.
const SCREEN_SERVERS: &[&str] = &["kimi-cu", "kimi_cu", "computer-use", "computer_use"];

/// Keys under which a screen tool names the app it acts on.
const APP_KEYS: &[&str] = &["app", "app_name", "appName", "application"];

/// Not a tool: the Codex adapter reports a server that failed to start as a
/// failed call named `mcp__<server>__startup`.
const STARTUP: &str = "startup";

/// Longest stretch of one string the scan reads; a script can run long.
const MAX_SCANNED: usize = 64 * 1024;

/// How deep into a call's input the scan looks.
const MAX_DEPTH: usize = 8;

/// The same call is written again only after this long.
const REPORT_SPACING: Duration = Duration::from_millis(750);

/// One call that acts on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCall {
    /// The tool: `click`, `type_text`, `get_app_state`, …
    pub tool: String,
    /// The app it acts on, when the call names one.
    pub app: Option<String>,
}

/// The screen-control call a `tool_call` or `tool_call_update` carries, if any.
///
/// Recognised the way each adapter spells it: the Claude adapter names an MCP
/// tool `mcp__kimi-cu__click`; the Codex adapter titles it `mcp.kimi-cu.click`
/// with `{server, tool}` in its input; Codex's code mode calls it from a
/// script, `mcp__kimi_cu__click({app: "…"})`. A script counts only where it
/// calls the tool — a search for the name is not a call.
pub fn screen_call_in(update: &Value) -> Option<ScreenCall> {
    let input = update.get("rawInput");
    let tool = update
        .get("title")
        .and_then(Value::as_str)
        .and_then(tool_in_name)
        .or_else(|| update.get("_meta").and_then(|meta| tool_in_meta(meta, 0)))
        .or_else(|| input.and_then(|input| tool_in_input(input, 0)))?;
    let app = input.and_then(|input| app_in(input, 0));
    Some(ScreenCall { tool, app })
}

/// Report the screen-control call in this tool update, if it carries one.
pub fn notice(update: &Value) {
    let Some(call) = screen_call_in(update) else {
        return;
    };
    tracing::info!(
        target: "acp::computer_use",
        tool = %call.tool,
        app = call.app.as_deref().unwrap_or("-"),
        "agent is using the computer"
    );
    report(&call);
}

static LAST_REPORT: Mutex<Option<(Instant, ScreenCall)>> = Mutex::new(None);

/// Write the call where the desktop reads it. Silent when the desktop asked
/// for no reports, and for a repeat of the call just written.
fn report(call: &ScreenCall) {
    let Some(path) = std::env::var_os(COMPUTER_USE_FILE_ENV).filter(|path| !path.is_empty()) else {
        return;
    };
    if !claim_report(&LAST_REPORT, call, Instant::now()) {
        return;
    }
    let relay = std::env::var("BUZZ_RELAY_URL").ok();
    if let Err(error) = write_report(Path::new(&path), call, relay.as_deref(), unix_ms()) {
        tracing::debug!(target: "acp::computer_use", "screen-control report not written: {error}");
    }
}

/// Whether this call is due a write: always, unless it repeats the last one
/// within [`REPORT_SPACING`].
fn claim_report(
    last: &Mutex<Option<(Instant, ScreenCall)>>,
    call: &ScreenCall,
    now: Instant,
) -> bool {
    let Ok(mut last) = last.lock() else {
        return true;
    };
    if let Some((at, previous)) = last.as_ref() {
        if previous == call && now.saturating_duration_since(*at) < REPORT_SPACING {
            return false;
        }
    }
    *last = Some((now, call.clone()));
    true
}

/// Replace the report file whole, so the desktop never reads half of one.
pub fn write_report(
    path: &Path,
    call: &ScreenCall,
    relay: Option<&str>,
    at_ms: u64,
) -> std::io::Result<()> {
    let body = serde_json::json!({
        "tool": call.tool,
        "app": call.app,
        "relay": relay,
        "at_ms": at_ms,
    });
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, body.to_string())?;
    std::fs::rename(&partial, path)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// `mcp__kimi-cu__click` or `mcp.kimi-cu.click` → `click`.
fn tool_in_name(name: &str) -> Option<String> {
    let name = name.trim();
    SCREEN_SERVERS.iter().find_map(|server| {
        [format!("mcp__{server}__"), format!("mcp.{server}.")]
            .iter()
            .find_map(|lead| name.strip_prefix(lead.as_str()))
            .map(tool_word)
            .filter(|tool| !tool.is_empty() && *tool != STARTUP)
            .map(str::to_string)
    })
}

/// The tool named in the adapter's own metadata (`_meta.claudeCode.toolName`).
fn tool_in_meta(meta: &Value, depth: usize) -> Option<String> {
    if depth > MAX_DEPTH {
        return None;
    }
    match meta {
        Value::String(text) => tool_in_name(text),
        Value::Object(fields) => fields
            .values()
            .find_map(|field| tool_in_meta(field, depth + 1)),
        Value::Array(items) => items.iter().find_map(|item| tool_in_meta(item, depth + 1)),
        _ => None,
    }
}

/// The tool a call's input invokes: `{server, tool}` from the Codex adapter,
/// or a script calling `mcp__<server>__<tool>(`.
fn tool_in_input(input: &Value, depth: usize) -> Option<String> {
    if depth > MAX_DEPTH {
        return None;
    }
    match input {
        Value::String(text) => tool_called_in_script(text),
        Value::Object(fields) => {
            let server = fields.get("server").and_then(Value::as_str);
            let tool = fields.get("tool").and_then(Value::as_str);
            if let (Some(server), Some(tool)) = (server, tool) {
                if SCREEN_SERVERS.contains(&server) && !tool.is_empty() && tool != STARTUP {
                    return Some(tool.to_string());
                }
            }
            fields
                .values()
                .find_map(|field| tool_in_input(field, depth + 1))
        }
        Value::Array(items) => items.iter().find_map(|item| tool_in_input(item, depth + 1)),
        _ => None,
    }
}

/// The first screen tool a script calls: `mcp__kimi_cu__click(`.
fn tool_called_in_script(script: &str) -> Option<String> {
    let script = scanned(script);
    SCREEN_SERVERS.iter().find_map(|server| {
        let lead = format!("mcp__{server}__");
        script.match_indices(lead.as_str()).find_map(|(at, _)| {
            let rest = &script[at + lead.len()..];
            let tool = tool_word(rest);
            let called = rest[tool.len()..].trim_start().starts_with('(');
            (called && !tool.is_empty() && tool != STARTUP).then(|| tool.to_string())
        })
    })
}

/// The app a call names: an `app` field, or `app: "…"` in a script.
fn app_in(input: &Value, depth: usize) -> Option<String> {
    if depth > MAX_DEPTH {
        return None;
    }
    match input {
        Value::String(text) => app_in_script(text),
        Value::Object(fields) => APP_KEYS
            .iter()
            .find_map(|key| fields.get(*key).and_then(Value::as_str).and_then(app_name))
            .or_else(|| fields.values().find_map(|field| app_in(field, depth + 1))),
        Value::Array(items) => items.iter().find_map(|item| app_in(item, depth + 1)),
        _ => None,
    }
}

/// `app: "Video Studio"` (or `'…'`, or a quoted key) in a script.
fn app_in_script(script: &str) -> Option<String> {
    let script = scanned(script);
    script.match_indices("app").find_map(|(at, _)| {
        let before = script[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        let rest = script[at + 3..].trim_start_matches(['"', '\'']);
        let rest = rest.trim_start().strip_prefix(':')?.trim_start();
        let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let value = &rest[1..];
        let end = value.find(quote)?;
        app_name(&value[..end])
    })
}

/// An app name worth showing: non-empty and short.
fn app_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.chars().count() <= 80).then(|| name.to_string())
}

/// The leading `[A-Za-z0-9_]` run of `text`.
fn tool_word(text: &str) -> &str {
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    &text[..end]
}

/// At most [`MAX_SCANNED`] bytes of `text`, cut on a character boundary.
fn scanned(text: &str) -> &str {
    if text.len() <= MAX_SCANNED {
        return text;
    }
    let mut end = MAX_SCANNED;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(tool: &str, app: Option<&str>) -> Option<ScreenCall> {
        Some(ScreenCall {
            tool: tool.to_string(),
            app: app.map(str::to_string),
        })
    }

    #[test]
    fn the_claude_adapter_names_the_tool_in_its_title() {
        let update = json!({
            "sessionUpdate": "tool_call",
            "title": "mcp__kimi-cu__click",
            "rawInput": {"app": "Video Studio", "index": 4}
        });
        assert_eq!(screen_call_in(&update), call("click", Some("Video Studio")));
    }

    #[test]
    fn the_claude_computer_use_server_counts_too() {
        let update = json!({
            "sessionUpdate": "tool_call",
            "title": "Screenshot",
            "_meta": {"claudeCode": {"toolName": "mcp__computer-use__screenshot"}},
            "rawInput": {}
        });
        assert_eq!(screen_call_in(&update), call("screenshot", None));
    }

    #[test]
    fn the_codex_adapter_titles_it_with_dots_and_carries_server_and_tool() {
        let update = json!({
            "sessionUpdate": "tool_call",
            "title": "mcp.kimi-cu.type_text",
            "kind": "execute",
            "rawInput": {
                "server": "kimi-cu",
                "tool": "type_text",
                "arguments": {"app": "Safari", "text": "hello"}
            }
        });
        assert_eq!(screen_call_in(&update), call("type_text", Some("Safari")));
    }

    #[test]
    fn an_update_that_only_carries_the_input_is_still_recognised() {
        let update = json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "c1",
            "rawInput": {"server": "kimi-cu", "tool": "get_app_state", "arguments": {"pid": 84254}}
        });
        assert_eq!(screen_call_in(&update), call("get_app_state", None));
    }

    #[test]
    fn a_script_that_calls_the_tool_counts() {
        let update = json!({
            "sessionUpdate": "tool_call",
            "title": "exec",
            "rawInput": {"code": "const s = await mcp__kimi_cu__get_app_state({ app: \"Video Studio\" });\nawait mcp__kimi_cu__click({app: 'Video Studio', index: 7});"}
        });
        assert_eq!(
            screen_call_in(&update),
            call("get_app_state", Some("Video Studio"))
        );
    }

    #[test]
    fn a_script_that_only_mentions_the_tools_does_not() {
        let update = json!({
            "sessionUpdate": "tool_call",
            "title": "exec",
            "rawInput": {"code": "text(ALL_TOOLS.filter(x => /kimi/.test(x.name))); // see mcp__kimi_cu__click docs"}
        });
        assert_eq!(screen_call_in(&update), None);
    }

    #[test]
    fn shell_commands_and_other_tools_are_not_screen_calls() {
        for update in [
            json!({"sessionUpdate": "tool_call", "title": "bash", "rawInput": {"command": "rg kimi-cu src/"}}),
            json!({"sessionUpdate": "tool_call", "title": "mcp.buzz-mcp.send_message", "rawInput": {"server": "buzz-mcp", "tool": "send_message"}}),
            json!({"sessionUpdate": "tool_call", "title": "mcp__kimi-cu-docs__click"}),
        ] {
            assert_eq!(screen_call_in(&update), None, "{update}");
        }
    }

    #[test]
    fn a_server_that_failed_to_start_is_not_a_call() {
        let update = json!({
            "sessionUpdate": "tool_call",
            "title": "mcp__kimi-cu__startup",
            "status": "failed"
        });
        assert_eq!(screen_call_in(&update), None);
    }

    #[test]
    fn app_names_are_read_from_scripts_only_as_whole_keys() {
        assert_eq!(
            app_in_script("run({ app: \"Finder\" })").as_deref(),
            Some("Finder")
        );
        assert_eq!(
            app_in_script("{\"app\": \"Notes\"}").as_deref(),
            Some("Notes")
        );
        assert_eq!(app_in_script("run({ myapp: \"Finder\" })"), None);
        assert_eq!(app_in_script("apply: 'x'"), None);
    }

    #[test]
    fn the_same_call_is_written_once_per_spacing_and_a_new_one_at_once() {
        let last = Mutex::new(None);
        let start = Instant::now();
        let click = ScreenCall {
            tool: "click".into(),
            app: Some("Finder".into()),
        };
        let typing = ScreenCall {
            tool: "type_text".into(),
            app: Some("Finder".into()),
        };
        assert!(claim_report(&last, &click, start));
        assert!(!claim_report(
            &last,
            &click,
            start + Duration::from_millis(100)
        ));
        assert!(claim_report(
            &last,
            &typing,
            start + Duration::from_millis(200)
        ));
        assert!(claim_report(
            &last,
            &typing,
            start + Duration::from_millis(1_000)
        ));
    }

    #[test]
    fn the_report_file_is_replaced_whole() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent.json");
        let click = ScreenCall {
            tool: "click".into(),
            app: Some("Video Studio".into()),
        };
        write_report(
            &path,
            &click,
            Some("wss://relay.example"),
            1_700_000_000_000,
        )
        .expect("written");
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
        assert_eq!(
            written,
            json!({"tool": "click", "app": "Video Studio", "relay": "wss://relay.example", "at_ms": 1_700_000_000_000u64})
        );
        assert!(!path.with_extension("json.partial").exists());
    }
}
