//! Put the agent that is using the computer above every window.
//!
//! Each agent's harness writes its latest screen-control call — a click, a
//! keystroke, a screenshot — to `agents/computer-use/<pubkey>.json` (see
//! buzz-acp's `computer_use`). This watcher reads those files twice a second.
//! While any agent has touched the screen within [`ACTIVE_FOR_MS`], a bar
//! floats above every app naming the agent and what it is doing, with a Stop
//! button; the owner also gets one notification as each agent starts, the
//! way Claude and Codex announce their own computer use.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// Env var the harness reads to learn where to write its reports.
pub const COMPUTER_USE_FILE_ENV: &str = "BUZZ_ACP_COMPUTER_USE_FILE";

/// The bar's window.
pub const BAR_LABEL: &str = "computer-use";

/// How long after its last call an agent still counts as at the controls:
/// long enough to bridge the model thinking between two clicks.
pub const ACTIVE_FOR_MS: u64 = 15_000;

/// A report stamped further ahead than this is a clock problem, not activity.
const FUTURE_SLACK_MS: u64 = 5_000;

const POLL: Duration = Duration::from_millis(500);
const STALE_AFTER: Duration = Duration::from_secs(60);
const NAMES_REFRESH: Duration = Duration::from_secs(10);
const BAR_WIDTH: f64 = 560.0;
const BAR_ROW: f64 = 48.0;
const BAR_TOP: f64 = 44.0;
const MAX_ROWS: usize = 4;

/// One harness's latest screen-control call, as it wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Report {
    pub tool: String,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub relay: Option<String>,
    pub at_ms: u64,
}

/// An agent at the controls, as the bar shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveAgent {
    pub pubkey: String,
    pub name: String,
    /// What it is doing, in the owner's words.
    pub action: String,
    pub app: Option<String>,
    pub relay_url: Option<String>,
    /// Driving the Mac (clicks, keys), not only looking at it.
    pub controls: bool,
    pub at_ms: u64,
}

/// Where the harnesses write their reports.
pub fn reports_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = crate::managed_agents::managed_agents_base_dir(app)?.join("computer-use");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create computer-use dir: {error}"))?;
    Ok(dir)
}

/// The report file of one agent's harness.
pub fn report_path(app: &AppHandle, pubkey: &str) -> Result<PathBuf, String> {
    Ok(reports_dir(app)?.join(report_file_name(pubkey)?))
}

/// `<pubkey>.json`, for a hex pubkey only: the name can never leave the folder.
pub fn report_file_name(pubkey: &str) -> Result<String, String> {
    if pubkey.is_empty() || !pubkey.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("agent pubkey is not hex".to_string());
    }
    Ok(format!("{}.json", pubkey.to_ascii_lowercase()))
}

/// What a screen tool does, in the owner's words, and whether it drives the
/// Mac rather than only looking at it. An unknown tool counts as driving.
pub fn describe(tool: &str) -> (String, bool) {
    let tool_lower = tool.to_ascii_lowercase();
    let has = |part: &str| tool_lower.contains(part);
    let (action, controls) = if has("click") || tool_lower == "perform_secondary_action" {
        ("ينقر", true)
    } else if has("type") || tool_lower == "set_value" {
        ("يكتب", true)
    } else if has("key") {
        ("يضغط أزرار", true)
    } else if has("scroll") {
        ("يمرّر", true)
    } else if has("drag") {
        ("يسحب", true)
    } else if has("select") {
        ("يحدد نصاً", true)
    } else if has("mouse") || has("move") {
        ("يحرك الماوس", true)
    } else if has("open") || has("launch") {
        ("يفتح تطبيقاً", true)
    } else if has("screenshot") || has("state") || has("zoom") {
        ("يشوف الشاشة", false)
    } else if has("list") {
        ("يشوف التطبيقات المفتوحة", false)
    } else {
        return (tool.to_string(), true);
    };
    (action.to_string(), controls)
}

/// The agents at the controls at `now_ms`, most recent first.
pub fn active_agents(
    reports: &[(String, Report)],
    names: &HashMap<String, String>,
    now_ms: u64,
) -> Vec<ActiveAgent> {
    let mut active: Vec<ActiveAgent> = reports
        .iter()
        .filter(|(_, report)| {
            report.at_ms <= now_ms.saturating_add(FUTURE_SLACK_MS)
                && now_ms.saturating_sub(report.at_ms) <= ACTIVE_FOR_MS
        })
        .map(|(pubkey, report)| {
            let (action, controls) = describe(&report.tool);
            ActiveAgent {
                pubkey: pubkey.clone(),
                name: names
                    .get(pubkey)
                    .cloned()
                    .unwrap_or_else(|| format!("Agent {}", &pubkey[..pubkey.len().min(8)])),
                action,
                app: report.app.clone(),
                relay_url: report.relay.clone(),
                controls,
                at_ms: report.at_ms,
            }
        })
        .collect();
    active.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then_with(|| a.pubkey.cmp(&b.pubkey)));
    active
}

/// The notification for an agent that just took the controls.
pub fn announcement(agent: &ActiveAgent) -> (String, String) {
    let verb = if agent.controls {
        "يتحكم بجهازك"
    } else {
        "يشوف شاشتك"
    };
    let body = match &agent.app {
        Some(app) => format!("{} في {app}", agent.action),
        None => agent.action.clone(),
    };
    (format!("{} {verb}", agent.name), body)
}

/// Every report in `dir`, keyed by the agent's pubkey. Unreadable or
/// half-written files are skipped; the next look reads them whole.
pub fn read_reports(dir: &Path) -> Vec<(String, Report)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                return None;
            }
            // A report untouched for a minute cannot be recent activity;
            // leaving it unread keeps the look cheap while nobody is at the Mac.
            let modified = entry.metadata().ok()?.modified().ok()?;
            if modified.elapsed().is_ok_and(|age| age > STALE_AFTER) {
                return None;
            }
            let pubkey = path.file_stem()?.to_str()?.to_ascii_lowercase();
            let text = std::fs::read_to_string(&path).ok()?;
            let report = serde_json::from_str::<Report>(&text).ok()?;
            Some((pubkey, report))
        })
        .collect()
}

/// Start watching; runs for the life of the app.
pub fn start(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("computer-use-watch".into())
        .spawn(move || watch(&app));
    if let Err(error) = spawned {
        eprintln!("buzz-desktop: computer-use watch not started: {error}");
    }
}

fn watch(app: &AppHandle) {
    let dir = match reports_dir(app) {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("buzz-desktop: computer-use watch has no folder: {error}");
            return;
        }
    };
    let mut names: HashMap<String, String> = HashMap::new();
    let mut names_read: Option<Instant> = None;
    let mut on_bar: BTreeSet<String> = BTreeSet::new();
    let mut shown = String::from("[]");
    loop {
        std::thread::sleep(POLL);
        let reports = read_reports(&dir);
        let unnamed = reports
            .iter()
            .any(|(pubkey, _)| !names.contains_key(pubkey));
        if unnamed && names_read.is_none_or(|at| at.elapsed() >= NAMES_REFRESH) {
            names = agent_names(app);
            names_read = Some(Instant::now());
        }
        let active = active_agents(&reports, &names, unix_ms());
        for agent in active
            .iter()
            .filter(|agent| !on_bar.contains(&agent.pubkey))
        {
            announce(agent);
        }
        on_bar = active.iter().map(|agent| agent.pubkey.clone()).collect();
        let payload = serde_json::to_string(&active).unwrap_or_else(|_| "[]".to_string());
        if payload != shown {
            show_bar(app, active.len(), &payload);
            shown = payload;
        }
    }
}

fn agent_names(app: &AppHandle) -> HashMap<String, String> {
    crate::managed_agents::load_managed_agents(app)
        .map(|records| {
            records
                .into_iter()
                .map(|record| (record.pubkey.to_ascii_lowercase(), record.name))
                .collect()
        })
        .unwrap_or_default()
}

fn announce(agent: &ActiveAgent) {
    let (title, body) = announcement(agent);
    #[cfg(target_os = "macos")]
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::macos_notifications::show(title, Some(body), None).await {
            eprintln!("buzz-desktop: computer-use notification not shown: {error}");
        }
    });
    #[cfg(not(target_os = "macos"))]
    let _ = (title, body);
}

/// Show, update or close the bar for `rows` agents described by `payload`.
fn show_bar(app: &AppHandle, rows: usize, payload: &str) {
    let existing = app.get_webview_window(BAR_LABEL);
    if rows == 0 {
        if let Some(window) = existing {
            let _ = window.close();
        }
        return;
    }
    let height = 4.0 + BAR_ROW * rows.min(MAX_ROWS) as f64;
    if let Some(window) = existing {
        let _ = window.set_size(tauri::LogicalSize::new(BAR_WIDTH, height));
        let _ = window.eval(format!(
            "window.cortexComputerUse && window.cortexComputerUse({payload})"
        ));
        return;
    }
    let (x, y) = bar_position(app);
    let built =
        WebviewWindowBuilder::new(app, BAR_LABEL, WebviewUrl::App("computer-use.html".into()))
            .title("Cortex Max")
            .inner_size(BAR_WIDTH, height)
            .position(x, y)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .visible_on_all_workspaces(true)
            .skip_taskbar(true)
            .focused(false)
            .accept_first_mouse(true)
            .initialization_script(format!("window.__CORTEX_COMPUTER_USE__ = {payload};"))
            .build();
    if let Err(error) = built {
        eprintln!("buzz-desktop: computer-use bar not shown: {error}");
    }
}

/// Top centre of the main display, clear of the menu bar.
fn bar_position(app: &AppHandle) -> (f64, f64) {
    let Ok(Some(monitor)) = app.primary_monitor() else {
        return (400.0, BAR_TOP);
    };
    let scale = monitor.scale_factor();
    let size = monitor.size().to_logical::<f64>(scale);
    let origin = monitor.position().to_logical::<f64>(scale);
    (
        origin.x + ((size.width - BAR_WIDTH) / 2.0).max(0.0),
        origin.y + BAR_TOP,
    )
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Stop an agent from the bar. Its harness ends, and with it the screen tool
/// it started; the bar clears on the next look.
#[tauri::command]
pub fn stop_computer_use_agent(
    pubkey: String,
    relay_url: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    let relay_url = relay_url
        .filter(|relay| !relay.is_empty())
        .ok_or("the agent's community is unknown; stop it from the Agents page")?;
    crate::managed_agents::stop_managed_agent_runtime(pubkey.clone(), relay_url, app.clone())?;
    if let Ok(path) = report_path(&app, &pubkey) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(tool: &str, app: Option<&str>, at_ms: u64) -> Report {
        Report {
            tool: tool.to_string(),
            app: app.map(str::to_string),
            relay: Some("wss://relay.example".to_string()),
            at_ms,
        }
    }

    fn names() -> HashMap<String, String> {
        HashMap::from([("aa11".to_string(), "Syntax".to_string())])
    }

    #[test]
    fn clicks_and_keys_drive_the_mac_while_screenshots_only_look() {
        assert_eq!(describe("click"), ("ينقر".to_string(), true));
        assert_eq!(describe("left_click"), ("ينقر".to_string(), true));
        assert_eq!(describe("type_text"), ("يكتب".to_string(), true));
        assert_eq!(describe("set_value"), ("يكتب".to_string(), true));
        assert_eq!(describe("press_key"), ("يضغط أزرار".to_string(), true));
        assert_eq!(
            describe("get_app_state"),
            ("يشوف الشاشة".to_string(), false)
        );
        assert_eq!(describe("screenshot"), ("يشوف الشاشة".to_string(), false));
        assert_eq!(
            describe("list_apps"),
            ("يشوف التطبيقات المفتوحة".to_string(), false)
        );
        assert_eq!(describe("teleport"), ("teleport".to_string(), true));
    }

    #[test]
    fn an_agent_stays_on_the_bar_for_the_active_window_then_leaves() {
        let reports = vec![(
            "aa11".to_string(),
            report("click", Some("Video Studio"), 100_000),
        )];
        let at = |now| active_agents(&reports, &names(), now);
        assert_eq!(at(100_000).len(), 1);
        assert_eq!(at(100_000 + ACTIVE_FOR_MS).len(), 1);
        assert!(at(100_001 + ACTIVE_FOR_MS).is_empty());
        let agent = &at(101_000)[0];
        assert_eq!(agent.name, "Syntax");
        assert_eq!(agent.action, "ينقر");
        assert_eq!(agent.app.as_deref(), Some("Video Studio"));
        assert_eq!(agent.relay_url.as_deref(), Some("wss://relay.example"));
        assert!(agent.controls);
    }

    #[test]
    fn a_report_from_the_future_is_not_activity() {
        let reports = vec![("aa11".to_string(), report("click", None, 200_000))];
        assert_eq!(active_agents(&reports, &names(), 196_000).len(), 1);
        assert!(active_agents(&reports, &names(), 150_000).is_empty());
    }

    #[test]
    fn the_most_recent_agent_comes_first_and_unknown_agents_get_a_short_name() {
        let reports = vec![
            ("aa11".to_string(), report("click", None, 100_000)),
            (
                "bb22ccdd99".to_string(),
                report("screenshot", None, 104_000),
            ),
        ];
        let active = active_agents(&reports, &names(), 105_000);
        assert_eq!(active[0].name, "Agent bb22ccdd");
        assert!(!active[0].controls);
        assert_eq!(active[1].name, "Syntax");
    }

    #[test]
    fn the_announcement_says_who_and_what() {
        let reports = vec![(
            "aa11".to_string(),
            report("type_text", Some("Safari"), 100_000),
        )];
        let agent = &active_agents(&reports, &names(), 100_000)[0];
        assert_eq!(
            announcement(agent),
            (
                "Syntax يتحكم بجهازك".to_string(),
                "يكتب في Safari".to_string()
            )
        );
        let looking = vec![("aa11".to_string(), report("get_app_state", None, 100_000))];
        let agent = &active_agents(&looking, &names(), 100_000)[0];
        assert_eq!(
            announcement(agent),
            ("Syntax يشوف شاشتك".to_string(), "يشوف الشاشة".to_string())
        );
    }

    #[test]
    fn reports_are_read_by_pubkey_and_broken_files_are_skipped() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("AA11.json"),
            r#"{"tool":"click","app":"Finder","relay":"wss://r","at_ms":5}"#,
        )
        .expect("write");
        std::fs::write(dir.path().join("bb22.json"), "{\"tool\":").expect("write");
        std::fs::write(dir.path().join("cc33.json.partial"), "{}").expect("write");
        let reports = read_reports(dir.path());
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].0, "aa11");
        assert_eq!(reports[0].1, {
            let mut expected = report("click", Some("Finder"), 5);
            expected.relay = Some("wss://r".to_string());
            expected
        });
    }

    #[test]
    fn only_hex_pubkeys_name_a_report_file() {
        assert_eq!(report_file_name("AA11ff").as_deref(), Ok("aa11ff.json"));
        for pubkey in ["", "../escape", "aa11/..", "zz", "aa11.json"] {
            assert!(report_file_name(pubkey).is_err(), "{pubkey}");
        }
    }
}
