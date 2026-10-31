//! The provider limits an owner actually hits, read from each runtime's own
//! reporting.
//!
//! The spend dashboard answers "what did this cost", in dollars each harness
//! reports for its own turns. On a subscription that number is API-equivalent
//! value, not a bill — the thing that actually stops the team mid-run is the
//! provider's own window: Claude's 5-hour and weekly caps, Codex's weekly cap.
//! Those were only visible by leaving the app, so a run could die against a
//! ceiling the owner had no way to watch.
//!
//! Neither figure is estimated here. Each runtime already reports its own
//! limits and this module only reads them:
//!
//! * Claude, by asking the installed CLI (`claude -p "/usage"`). The CLI holds
//!   its own credentials and answers for the signed-in account; this module
//!   never reads, copies or forwards a token. The call costs one small request,
//!   so results are cached for [`CACHE_TTL`] rather than fetched per render.
//! * Codex, by asking its own app-server (`account/rateLimits/read`). Codex
//!   holds its sign-in and makes the request itself; this module speaks to it
//!   over stdio and never sees a token. No turn runs, so asking spends none of
//!   the allowance it reports. When Codex cannot be asked, the fallback is the
//!   `rate_limits` block it writes into its session rollouts under
//!   `~/.codex/sessions` — a local file that is only as fresh as its last turn.
//!
//! A runtime that cannot be read reports why, as [`LimitGroup::unavailable`].
//! It never reports zero: "no limit data" and "no limit used" look identical on
//! a progress bar and mean opposite things.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// How long a fetched snapshot is reused. The Claude read costs a request, and
/// these windows move over hours, so a per-render fetch would spend the very
/// limit it reports on.
const CACHE_TTL: Duration = Duration::from_secs(300);

/// One limit window, as the provider describes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    /// The provider's own name for this window, e.g. "Session" or "Week".
    pub label: String,
    /// Percent of the window consumed, 0-100.
    pub used_percent: f64,
    /// When the window resets, as the provider stated it. Free text for Claude
    /// (which answers in the owner's timezone) and a formatted instant for
    /// Codex; `None` when the provider did not say.
    pub resets: Option<String>,
    /// Unix seconds, when the provider gave an instant rather than prose.
    pub resets_at: Option<i64>,
    /// How long the whole window is, in minutes.
    ///
    /// Codex states this outright. Claude does not, so it is derived from the
    /// CLI's own wording — "Current session" is its 5-hour window and "Current
    /// week" its weekly one. `None` when neither source says, which is what
    /// stops a forecast being drawn over a window of unknown length.
    pub window_minutes: Option<i64>,
    /// The window's reset time has passed, so this percentage describes a
    /// window that no longer exists.
    ///
    /// This is only reachable for a runtime read from a file it writes itself:
    /// if Codex has not taken a turn since its window rolled over, the last
    /// reading it wrote is all there is. Reporting that stale number as
    /// current would tell an owner they are still locked out when the limit
    /// has in fact reset — the single most costly thing this panel could get
    /// wrong.
    pub stale: bool,
    /// The model this window is scoped to, when the provider reports one.
    ///
    /// Claude publishes a separate weekly window per premium model alongside
    /// an account-wide one, so a single model can be spent while the account
    /// still has room. `None` means the window covers the whole account —
    /// which is the difference between "switch model" and "switch provider",
    /// so it is kept as a field rather than re-read out of `label`.
    #[serde(default)]
    pub model: Option<String>,
}

/// Every window one runtime reports, or the reason there are none.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitGroup {
    /// Runtime id, matching the harness ids the usage rollup uses.
    pub runtime: String,
    /// Display name, e.g. "Claude".
    pub label: String,
    pub windows: Vec<LimitWindow>,
    /// Plan name when the runtime states one.
    pub plan: Option<String>,
    /// A note worth showing beside the bars, e.g. a credit balance.
    pub note: Option<String>,
    /// Why this runtime reports nothing. `None` when it reported normally.
    pub unavailable: Option<String>,
    /// How old the underlying reading is, in seconds, when it came from a file
    /// the runtime wrote rather than a live call.
    pub age_seconds: Option<i64>,
}

impl LimitGroup {
    fn unavailable(runtime: &str, label: &str, reason: impl Into<String>) -> Self {
        Self {
            runtime: runtime.to_string(),
            label: label.to_string(),
            windows: Vec::new(),
            plan: None,
            note: None,
            unavailable: Some(reason.into()),
            age_seconds: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Claude
// ---------------------------------------------------------------------------

/// Parse the CLI's usage report.
///
/// The report is prose meant for a terminal, so this reads only the lines that
/// state a percentage and keeps the provider's own wording for the rest. A
/// line it does not recognize is skipped rather than guessed at.
///
/// ```text
/// Current session: 33% used · resets Sep 19 at 4pm (Europe/Istanbul)
/// Current week (all models): 67% used · resets Sep 23 at 9am (Europe/Istanbul)
/// ```
fn parse_claude_usage(output: &str) -> Vec<LimitWindow> {
    let mut windows = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("Current ") else {
            continue;
        };
        let Some((name, tail)) = rest.split_once(':') else {
            continue;
        };
        let tail = tail.trim();
        let Some((percent_text, after)) = tail.split_once('%') else {
            continue;
        };
        let Ok(used_percent) = percent_text.trim().parse::<f64>() else {
            continue;
        };
        // "resets Sep 23 at 9am (Europe/Istanbul)" — kept verbatim, because the
        // CLI already rendered it in the owner's own timezone.
        let resets = after
            .split_once("resets")
            .map(|(_, when)| when.trim().to_string())
            .filter(|when| !when.is_empty());

        windows.push(LimitWindow {
            label: claude_window_label(name.trim()),
            used_percent: used_percent.clamp(0.0, 100.0),
            window_minutes: claude_window_minutes(name.trim()),
            resets_at: resets.as_deref().and_then(resolve_reset_epoch),
            resets,
            // Claude is read live on every fetch, so it is never stale.
            stale: false,
            model: claude_window_model(name.trim()),
        });
    }
    windows
}

/// The month, day, hour and minute stated in a Claude reset clause.
///
/// The CLI writes prose — "Sep 23 at 8:59am (Europe/Istanbul)" — with no year and
/// no offset. This reads only what is actually there; turning it into an
/// instant is [`nearest_future`]'s job, and choosing a timezone is the
/// caller's.
fn parse_reset_parts(text: &str) -> Option<(u32, u32, u32, u32)> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];

    let cleaned = text.trim().to_ascii_lowercase();
    // Drop the trailing "(Europe/Istanbul)" — it names the zone the CLI already
    // rendered into, which is the machine's own.
    let cleaned = cleaned.split('(').next()?.trim();
    let mut words = cleaned.split_whitespace();

    let month_word = words.next()?;
    let month = MONTHS
        .iter()
        .position(|name| month_word.starts_with(name))? as u32
        + 1;
    let day: u32 = words.next()?.trim_end_matches(',').parse().ok()?;

    // "at 8:59am" — the "at" is optional so a shorter phrasing still parses.
    let clock = match words.next()? {
        "at" => words.next()?,
        other => other,
    };

    let (clock, pm) = match () {
        _ if clock.ends_with("pm") => (clock.trim_end_matches("pm"), true),
        _ if clock.ends_with("am") => (clock.trim_end_matches("am"), false),
        _ => return None,
    };

    let (hour_text, minute_text) = clock.split_once(':').unwrap_or((clock, "0"));
    let hour12: u32 = hour_text.parse().ok()?;
    let minute: u32 = minute_text.parse().ok()?;
    if !(1..=12).contains(&hour12) || minute > 59 {
        return None;
    }

    // 12am is midnight and 12pm is noon; neither follows the +12 rule.
    let hour = match (hour12, pm) {
        (12, false) => 0,
        (12, true) => 12,
        (hour, true) => hour + 12,
        (hour, false) => hour,
    };
    Some((month, day, hour, minute))
}

/// The nearest occurrence of `parts` that is not in the past.
///
/// The CLI omits the year, so a window resetting on Jan 2 read on Dec 30 means
/// next January, not the one ten months gone. Anything already past rolls
/// forward a year rather than reporting a countdown that has expired.
fn nearest_future(
    parts: (u32, u32, u32, u32),
    now: chrono::NaiveDateTime,
) -> Option<chrono::NaiveDateTime> {
    use chrono::{Datelike, NaiveDate};
    let (month, day, hour, minute) = parts;

    for year in [now.year(), now.year() + 1] {
        let Some(candidate) = NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, minute, 0))
        else {
            // Feb 29 in a common year: try the next one rather than failing.
            continue;
        };
        // A minute of slack, so a reset happening right now reads as due
        // rather than jumping a whole year into the future.
        if candidate >= now - chrono::Duration::minutes(1) {
            return Some(candidate);
        }
    }
    None
}

/// Turn a Claude reset clause into a unix instant in the machine's timezone.
///
/// `None` when the clause cannot be read, which leaves the UI showing the
/// CLI's own wording and no countdown — better than a countdown to a guess.
fn resolve_reset_epoch(text: &str) -> Option<i64> {
    use chrono::{Local, TimeZone};
    let parts = parse_reset_parts(text)?;
    let now = Local::now().naive_local();
    let naive = nearest_future(parts, now)?;
    // An ambiguous local time (the repeated hour of a DST fall-back) resolves
    // to the earlier instant, which is the conservative choice for a deadline.
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|when| when.timestamp())
}

/// How long a Claude window lasts, read off the CLI's own name for it.
///
/// Anything else yields `None` rather than a guess: a forecast drawn over the
/// wrong window length is worse than no forecast, because it reads as a fact.
fn claude_window_minutes(name: &str) -> Option<i64> {
    if name.starts_with("session") {
        return Some(5 * 60);
    }
    if name.starts_with("week") {
        return Some(7 * 24 * 60);
    }
    None
}

/// The model a Claude window is scoped to, or `None` for an account-wide one.
///
/// The CLI writes `week (Fable)` for a per-model window and
/// `week (all models)` for the account-wide one. Only the former names a model
/// that could be switched away from; reading "all models" as a model name
/// would make the account-wide limit look like one model's problem.
fn claude_window_model(name: &str) -> Option<String> {
    name.strip_prefix("week (")
        .and_then(|rest| rest.strip_suffix(')'))
        .filter(|model| *model != "all models")
        .map(str::to_string)
}

/// Shorten the CLI's phrasing for a narrow bar without inventing a meaning.
fn claude_window_label(name: &str) -> String {
    match name {
        "session" => "Session".to_string(),
        "week (all models)" => "Week · all models".to_string(),
        other => {
            // "week (Fable)" -> "Week · Fable"
            match other
                .strip_prefix("week (")
                .and_then(|r| r.strip_suffix(')'))
            {
                Some(model) => format!("Week · {model}"),
                None => {
                    let mut chars = other.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => other.to_string(),
                    }
                }
            }
        }
    }
}

fn read_claude_limits() -> LimitGroup {
    let mut command = Command::new("claude");
    command.args(["-p", "/usage"]);
    // A GUI app launched from Finder inherits none of the login shell's PATH,
    // so `claude` in ~/.local/bin is invisible to it. This is the same
    // augmented PATH the app already uses to find harness binaries; without it
    // the panel would report "not installed" on a machine where it plainly is.
    if let Some(path) = crate::managed_agents::readiness::cli_probe::augmented_path() {
        command.env("PATH", path);
    }
    let output = command.output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return LimitGroup::unavailable(
                "claude",
                "Claude",
                format!("could not run the claude CLI: {error}"),
            );
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().next().unwrap_or("the claude CLI failed");
        return LimitGroup::unavailable("claude", "Claude", reason.to_string());
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let windows = parse_claude_usage(&text);
    if windows.is_empty() {
        return LimitGroup::unavailable(
            "claude",
            "Claude",
            "the claude CLI reported no limit windows",
        );
    }
    LimitGroup {
        runtime: "claude".to_string(),
        label: "Claude".to_string(),
        windows,
        plan: text
            .contains("subscription")
            .then(|| "subscription".to_string()),
        note: None,
        unavailable: None,
        age_seconds: None,
    }
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

/// Name a Codex window by its length, since Codex reports minutes rather than
/// a label.
fn codex_window_label(window_minutes: i64) -> String {
    match window_minutes {
        m if m % 10_080 == 0 && m / 10_080 == 1 => "Week".to_string(),
        m if m % 10_080 == 0 => format!("{} weeks", m / 10_080),
        m if m % 1_440 == 0 && m / 1_440 == 1 => "Day".to_string(),
        m if m % 1_440 == 0 => format!("{} days", m / 1_440),
        m if m % 60 == 0 => format!("{} hours", m / 60),
        m => format!("{m} minutes"),
    }
}

/// Pull the windows out of one `rate_limits` object.
///
/// `now` is unix seconds, taken as an argument so the stale check is testable
/// without waiting a week for a real window to roll over.
fn parse_codex_rate_limits(value: &serde_json::Value, now: i64) -> Vec<LimitWindow> {
    ["primary", "secondary"]
        .iter()
        .filter_map(|slot| value.get(*slot))
        .filter(|slot| !slot.is_null())
        .filter_map(|slot| {
            let used_percent = slot.get("used_percent")?.as_f64()?;
            let window_minutes = slot.get("window_minutes").and_then(|v| v.as_i64());
            let resets_at = slot.get("resets_at").and_then(|v| v.as_i64());
            Some(LimitWindow {
                label: window_minutes.map_or_else(|| "Limit".to_string(), codex_window_label),
                window_minutes,
                used_percent: used_percent.clamp(0.0, 100.0),
                resets: resets_at.map(format_instant),
                resets_at,
                stale: resets_at.is_some_and(|resets_at| resets_at <= now),
                model: None,
            })
        })
        .collect()
}

/// Codex's usage limit, drawn spent, for the case where it reports no
/// rate-limit window because the account has nothing left to spend.
///
/// `primary` and `secondary` both go null once the account is out of credit:
/// there is no consumption rate to describe when nothing can be consumed.
/// Dropping the runtime from the strip at that point hides it in the one
/// moment it matters most — an exhausted provider is why the agents on it
/// stopped, and a missing bar reads as "fine" rather than "stopped".
///
/// This is a read figure, not a guess: the balance is stated in the rollout.
/// That is the line between a runtime worth showing and one worth omitting.
fn codex_credit_window(limits: &serde_json::Value) -> Option<LimitWindow> {
    let credits = limits.get("credits")?;
    // Unlimited credit has no ceiling to draw, and an account still holding a
    // balance has merely not been measured yet. Neither of those is spent.
    if credits.get("unlimited")?.as_bool()? || credits.get("has_credits")?.as_bool()? {
        return None;
    }
    Some(LimitWindow {
        // "Usage", not "Credits": what ran out is the plan's allowance. The zero
        // balance is only the add-on credit that would have carried past it,
        // and naming the bar after that made an exhausted plan read as a
        // missing purchase.
        label: "Usage".to_string(),
        window_minutes: None,
        used_percent: 100.0,
        // The reading itself carries no reset; the caller finds the one Codex
        // recorded elsewhere, and marks the bar stale once it has passed.
        resets: None,
        resets_at: None,
        stale: false,
        model: None,
    })
}

/// The note beside Codex's bars: what its credit balance says, when that adds
/// anything.
///
/// A credit balance is what keeps Codex working past a full window, so it
/// belongs next to the bar rather than in a separate screen. The wording
/// carries weight: `agent_leave` reads this note to tell an owner why an agent
/// stopped, and "no credit" is the phrase it looks for — so it is said only
/// when it is the reason.
///
/// `unwindowed` is whether the reading reported no rate-limit window at all,
/// the shape of a spent plan. Only then does a zero balance mean "no credit".
/// Beside a window with room left it is an add-on the owner never bought, and
/// saying "no credit" there sends every Codex agent on leave with a full week
/// in hand. `spent_and_reset` is a spent reading whose reset has since passed:
/// that account is not out of anything either.
fn codex_credit_note(
    limits: &serde_json::Value,
    unwindowed: bool,
    spent_and_reset: bool,
) -> Option<String> {
    let credits = limits.get("credits")?;
    if credits.get("unlimited")?.as_bool()? {
        return Some("unlimited credits".to_string());
    }
    let balance = credits.get("balance")?.as_str()?;
    if credits
        .get("has_credits")
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        if !unwindowed || spent_and_reset {
            return None;
        }
        return Some(format!("no credit left \u{2014} balance {balance}"));
    }
    Some(format!("credits: {balance}"))
}

/// The latest reset time Codex recorded that falls after `after`.
///
/// An account out of credit reports its rate-limit slots as null, so the
/// reading that says "spent" carries no reset time at all — the one number
/// the owner actually wants, which is when Codex comes back. Earlier readings
/// in the same week still carry it: the provider's own figure, from the
/// provider's own file.
///
/// `after` is the moment the account was read as spent. A reset later than
/// that is what ends it — and once that reset has itself passed, the account
/// is not spent any more, whatever the last reading said. A reset from before
/// the reading describes a window that was already over when it ran dry.
fn latest_reset_after(rollouts: &[PathBuf], after: i64) -> Option<i64> {
    let mut best: Option<i64> = None;
    for path in rollouts.iter().take(CODEX_ROLLOUTS_TO_TRY * 4) {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines().filter(|line| line.contains("\"rate_limits\"")) {
            let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(limits) = parsed.get("payload").and_then(|p| p.get("rate_limits")) else {
                continue;
            };
            for slot in ["primary", "secondary"] {
                let Some(at) = limits
                    .get(slot)
                    .and_then(|window| window.get("resets_at"))
                    .and_then(serde_json::Value::as_i64)
                else {
                    continue;
                };
                if at > after {
                    best = Some(best.map_or(at, |current| current.max(at)));
                }
            }
        }
    }
    best
}

/// Unix seconds now.
fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// Render a unix instant as a short local date, so the UI can show it without
/// re-deriving a timezone.
fn format_instant(epoch_seconds: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(epoch_seconds, 0).single() {
        Some(when) => when.format("%b %-d at %-I%P").to_string(),
        None => String::new(),
    }
}

/// How far back to look for a rollout that actually recorded a limit.
///
/// A session only writes `rate_limits` on a turn that made a request, so the
/// newest file is routinely silent — it was opened and never used. Stopping
/// there reports "Codex has not reported a limit yet" while the reading sits
/// one file behind, which is how an exhausted provider vanishes from the
/// panel. Ten crosses a run of short sessions without reading a history.
const CODEX_ROLLOUTS_TO_TRY: usize = 10;

/// Recent Codex rollouts, newest first by mtime.
///
/// Ordering is by modification time, not by name: a long session keeps being
/// appended to after a later one started, so its reading can be the fresher of
/// the two. The walk stays bounded to the most recent days rather than the
/// whole history, which runs to hundreds of files.
fn codex_rollouts_newest_first(sessions_dir: &Path) -> Vec<PathBuf> {
    fn newest_entries(dir: &Path, take: usize) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        names.sort();
        names.reverse();
        names.truncate(take);
        names
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for year in newest_entries(sessions_dir, 2) {
        for month in newest_entries(&year, 2) {
            for day in newest_entries(&month, 2) {
                files.extend(
                    newest_entries(&day, usize::MAX)
                        .into_iter()
                        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl")),
                );
            }
        }
    }
    files.sort_by_key(|path| {
        std::cmp::Reverse(
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .unwrap_or(UNIX_EPOCH),
        )
    });
    files
}

/// Scan one rollout for the last `rate_limits` it recorded.
#[cfg(test)]
fn last_rate_limits_in(path: &Path) -> Option<serde_json::Value> {
    last_reading_in(path).map(|(limits, _)| limits)
}

/// The last `rate_limits` a rollout recorded, with the unix time of the line
/// when it carries one.
fn last_reading_in(path: &Path) -> Option<(serde_json::Value, Option<i64>)> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .filter(|line| line.contains("\"rate_limits\""))
        .find_map(|line| {
            let parsed: serde_json::Value = serde_json::from_str(line).ok()?;
            let limits = parsed.get("payload")?.get("rate_limits")?;
            if limits.is_null() {
                return None;
            }
            let read_at = parsed
                .get("timestamp")
                .and_then(serde_json::Value::as_str)
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .map(|at| at.timestamp());
            Some((limits.clone(), read_at))
        })
}

fn read_codex_limits(home: &Path) -> LimitGroup {
    let sessions = home.join(".codex/sessions");
    if !sessions.is_dir() {
        return LimitGroup::unavailable("codex", "Codex", "no Codex sessions on this machine");
    }
    let rollouts = codex_rollouts_newest_first(&sessions);
    if rollouts.is_empty() {
        return LimitGroup::unavailable("codex", "Codex", "no Codex session files yet");
    }
    // Walk back until a file actually carries a reading: the newest rollout is
    // frequently a session that was opened and never used.
    let Some((rollout, limits, read_at)) = rollouts
        .into_iter()
        .take(CODEX_ROLLOUTS_TO_TRY)
        .find_map(|path| last_reading_in(&path).map(|(limits, at)| (path, limits, at)))
    else {
        return LimitGroup::unavailable(
            "codex",
            "Codex",
            "Codex has not reported a limit yet — it reports one after its next turn",
        );
    };

    let now = now_seconds();
    let mut windows = parse_codex_rate_limits(&limits, now);
    // No rate-limit window at all is the shape of a spent plan.
    let unwindowed = windows.is_empty();
    // Whether the account was read as spent and has since come back.
    let mut spent_and_reset = false;
    if unwindowed {
        let mut spent = codex_credit_window(&limits);
        if let Some(window) = spent.as_mut() {
            let rollouts = codex_rollouts_newest_first(&sessions);
            // Without the reading's own time, only a reset still ahead can be
            // trusted to end it.
            if let Some(at) = latest_reset_after(&rollouts, read_at.unwrap_or(now)) {
                window.resets_at = Some(at);
                window.resets = Some(format_instant(at));
                window.stale = at <= now;
                spent_and_reset = window.stale;
            }
        }
        windows.extend(spent);
    }
    if windows.is_empty() {
        return LimitGroup::unavailable("codex", "Codex", "Codex reported no limit windows");
    }

    let age_seconds = std::fs::metadata(&rollout)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|when| SystemTime::now().duration_since(when).ok())
        .map(|elapsed| elapsed.as_secs() as i64);

    let note = codex_credit_note(&limits, unwindowed, spent_and_reset);

    LimitGroup {
        runtime: "codex".to_string(),
        label: "Codex".to_string(),
        windows,
        plan: limits
            .get("plan_type")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        note,
        unavailable: None,
        age_seconds,
    }
}

// ---------------------------------------------------------------------------
// Codex, asked live
// ---------------------------------------------------------------------------

/// How long Codex may take to answer before its session log is read instead.
///
/// The answer is one request Codex makes to OpenAI; a slow network should cost
/// the panel its freshest figure, not its only one.
const CODEX_LIVE_TIMEOUT: Duration = Duration::from_secs(20);

/// Codex's limits: asked live, else read from its session log.
///
/// The log only changes when Codex takes a turn, so on its own it describes
/// the account as it stood at the last one. A spent week read that way stays
/// spent until someone runs Codex again, and a week that has reset shows no
/// percentage and no reset time at all — the owner was left asking the
/// provider's website what the app should have said. Asking answers for the
/// account as it is now.
fn current_codex_limits(home: &Path) -> LimitGroup {
    let answer = codex_binary()
        .ok_or_else(|| "no Codex binary found".to_string())
        .and_then(|codex| ask_codex_for_limits(&codex, CODEX_LIVE_TIMEOUT));
    match answer {
        Ok(answer) => {
            if let Some(group) = codex_group_from_live(&answer, now_seconds()) {
                return group;
            }
            tracing::info!("Codex answered with no limit window; reading its session log");
        }
        Err(reason) => {
            tracing::info!(
                "could not ask Codex for its limits ({reason}); reading its session log"
            );
        }
    }
    read_codex_limits(home)
}

/// The Codex binary to ask.
///
/// The one this app's Codex agents run — vendored inside the Codex ACP
/// adapter — so the panel reports the account those agents spend. A `codex`
/// the owner installed separately is the fallback.
fn codex_binary() -> Option<PathBuf> {
    crate::managed_agents::resolve_command("codex-acp")
        .and_then(|adapter| adapter.canonicalize().ok())
        // The adapter resolves to `<package>/dist/index.js`.
        .and_then(|adapter| Some(adapter.parent()?.parent()?.join("node_modules/@openai")))
        .and_then(|scope| vendored_codex_in(&scope))
        .or_else(|| crate::managed_agents::resolve_command("codex"))
}

/// `codex-<platform>/vendor/<target>/bin/codex` under the adapter's `@openai`
/// directory, whichever platform package npm installed.
fn vendored_codex_in(scope: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    std::fs::read_dir(scope)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("codex-"))
        .filter_map(|entry| std::fs::read_dir(entry.path().join("vendor")).ok())
        .flat_map(|targets| targets.flatten())
        .map(|target| target.path().join("bin").join(name))
        .find(|binary| binary.is_file())
}

/// Ask a Codex app-server for the account's rate limits.
///
/// Returns the `result` of `account/rateLimits/read`. The server is stopped as
/// soon as it answers, or once `timeout` has passed.
fn ask_codex_for_limits(codex: &Path, timeout: Duration) -> Result<serde_json::Value, String> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let mut child = Command::new(codex)
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start codex: {error}"))?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("codex started without its pipes".to_string());
    };

    // The reader lives on its own thread so the wait below can give up on a
    // server that never answers; a blocking read would hang the panel with it.
    let (lines, messages) = std::sync::mpsc::channel::<serde_json::Value>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(message) = serde_json::from_str(&line) {
                if lines.send(message).is_err() {
                    break;
                }
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let answer = converse_with_codex(&mut stdin, &messages, deadline);
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    answer
}

/// The handshake and the one question, in the order the app-server requires.
fn converse_with_codex(
    stdin: &mut impl std::io::Write,
    messages: &std::sync::mpsc::Receiver<serde_json::Value>,
    deadline: Instant,
) -> Result<serde_json::Value, String> {
    let initialize = serde_json::json!({
        "method": "initialize",
        "id": 0,
        "params": {
            "clientInfo": {
                "name": "cortex-max",
                "title": "Cortex Max",
                "version": env!("CARGO_PKG_VERSION"),
            },
        },
    });
    send_to_codex(stdin, &initialize)?;
    reply_from_codex(messages, 0, deadline)?;
    send_to_codex(stdin, &serde_json::json!({ "method": "initialized" }))?;
    send_to_codex(
        stdin,
        &serde_json::json!({ "method": "account/rateLimits/read", "id": 1 }),
    )?;
    reply_from_codex(messages, 1, deadline)
}

fn send_to_codex(
    stdin: &mut impl std::io::Write,
    message: &serde_json::Value,
) -> Result<(), String> {
    writeln!(stdin, "{message}")
        .and_then(|()| stdin.flush())
        .map_err(|error| format!("could not write to codex: {error}"))
}

/// The reply to request `id`, skipping the notifications Codex sends between.
fn reply_from_codex(
    messages: &std::sync::mpsc::Receiver<serde_json::Value>,
    id: i64,
    deadline: Instant,
) -> Result<serde_json::Value, String> {
    use std::sync::mpsc::RecvTimeoutError;
    loop {
        let message = messages
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| match error {
                RecvTimeoutError::Timeout => "codex did not answer in time".to_string(),
                RecvTimeoutError::Disconnected => "codex exited without answering".to_string(),
            })?;
        if message.get("id").and_then(serde_json::Value::as_i64) != Some(id) {
            continue;
        }
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("an error");
            return Err(format!("codex answered with {text}"));
        }
        return Ok(message
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null));
    }
}

/// Codex's live answer, reshaped as the `rate_limits` block its session log
/// carries, so one parser reads both and the two cannot drift apart.
///
/// They differ only in spelling: the app-server speaks camelCase, names the
/// window length `windowDurationMins`, and gives the balance as a number where
/// the log writes a string.
fn live_limits_as_logged(snapshot: &serde_json::Value) -> serde_json::Value {
    use serde_json::{json, Value};
    let window = |slot: Option<&Value>| match slot {
        Some(slot) if !slot.is_null() => json!({
            "used_percent": slot.get("usedPercent"),
            "window_minutes": slot.get("windowDurationMins"),
            "resets_at": slot.get("resetsAt"),
        }),
        _ => Value::Null,
    };
    let credits = match snapshot.get("credits") {
        Some(credits) if !credits.is_null() => json!({
            "has_credits": credits.get("hasCredits"),
            "unlimited": credits.get("unlimited"),
            "balance": credits.get("balance").map(|balance| match balance {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            }),
        }),
        _ => Value::Null,
    };
    json!({
        "primary": window(snapshot.get("primary")),
        "secondary": window(snapshot.get("secondary")),
        "credits": credits,
        "plan_type": snapshot.get("planType"),
    })
}

/// Codex's limits from its live answer to `account/rateLimits/read`.
///
/// `None` when the answer holds nothing to draw, so the session log speaks
/// instead of an empty panel.
fn codex_group_from_live(answer: &serde_json::Value, now: i64) -> Option<LimitGroup> {
    let snapshot = answer
        .get("rateLimits")
        .filter(|snapshot| !snapshot.is_null())?;
    let limits = live_limits_as_logged(snapshot);
    let mut windows = parse_codex_rate_limits(&limits, now);
    let unwindowed = windows.is_empty();
    if unwindowed {
        windows.extend(codex_credit_window(&limits));
    }
    if windows.is_empty() {
        return None;
    }
    // A reset the owner can spend is theirs to use once the bar runs out, so it
    // sits beside the bar. Codex's own count, not an inference.
    let free_resets = answer
        .get("rateLimitResetCredits")
        .and_then(|credits| credits.get("availableCount"))
        .and_then(serde_json::Value::as_i64)
        .filter(|count| *count > 0)
        .map(|count| match count {
            1 => "1 free reset available".to_string(),
            count => format!("{count} free resets available"),
        });
    let note = match (codex_credit_note(&limits, unwindowed, false), free_resets) {
        (Some(credit), Some(resets)) => Some(format!("{credit} \u{b7} {resets}")),
        (credit, resets) => credit.or(resets),
    };
    Some(LimitGroup {
        runtime: "codex".to_string(),
        label: "Codex".to_string(),
        windows,
        plan: limits
            .get("plan_type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        note,
        unavailable: None,
        // Asked just now rather than read back from a file.
        age_seconds: None,
    })
}

// ---------------------------------------------------------------------------
// Kimi
// ---------------------------------------------------------------------------

/// Kimi Code's state, which is "installed but silent about its limits".
///
/// Kimi is a first-class runtime here — it is tiered, scheduled, recycled and
/// counted like the others — but it publishes no limit anywhere this can read.
/// Its `/usage` exists only inside its own TUI, its session state carries no
/// quota, and its ACP mode offers none; the only remaining source would be its
/// OAuth token, which this app does not touch.
///
/// It is still listed, with that reason. A runtime the owner is actively
/// running should not simply be missing from the panel that says how much room
/// is left — absence reads as "not installed", which is a different and wrong
/// claim.
fn read_kimi_limits(home: &Path) -> Option<LimitGroup> {
    // Only speak up on a machine that actually has Kimi; elsewhere the note
    // would be noise about a runtime the owner does not use.
    let installed = home.join(".kimi-code/bin/kimi").exists();
    if !installed {
        return None;
    }
    // What was checked, so the next person does not repeat it: `kimi -p
    // "/usage"` answers with a description of the command rather than the
    // panel, because the CLI renders that panel in its interactive UI and
    // never routes it to the agent. Nothing under `~/.kimi-code` caches the
    // figures either — the session state carries no quota fields at all. The
    // numbers exist only in a live call the TUI makes with the owner's
    // sign-in token, and this app does not use that token.
    Some(LimitGroup::unavailable(
        "kimi",
        "Kimi",
        "Kimi renders its plan usage inside its own interactive CLI and keeps \
         no copy on this machine, so there is nothing here to read: its \
         non-interactive mode answers `/usage` with a description of the \
         command rather than the figures. Run `kimi` and type /usage to see \
         them. Set a daily limit for Kimi under Usage instead: its spend is \
         recorded like every other runtime.",
    ))
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

struct Cached {
    at: Instant,
    groups: Vec<LimitGroup>,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

/// Read every runtime's own limit reporting.
///
/// `refresh` skips the cache, for the button an owner presses when they want
/// the number now rather than within [`CACHE_TTL`].
#[tauri::command]
pub async fn get_provider_limits(refresh: Option<bool>) -> Result<Vec<LimitGroup>, String> {
    if refresh != Some(true) {
        if let Ok(cache) = CACHE.lock() {
            if let Some(cached) = cache.as_ref() {
                if cached.at.elapsed() < CACHE_TTL {
                    return Ok(cached.groups.clone());
                }
            }
        }
    }

    let groups = tokio::task::spawn_blocking(|| {
        let home = dirs::home_dir().unwrap_or_default();
        // Each waits on its own CLI and a round trip to its provider; neither
        // needs the other, so a refresh costs the slower of the two, not both.
        let (claude, codex) = std::thread::scope(|scope| {
            let codex = scope.spawn(|| current_codex_limits(&home));
            let claude = read_claude_limits();
            let codex = codex.join().unwrap_or_else(|_| {
                LimitGroup::unavailable("codex", "Codex", "reading Codex's limits failed")
            });
            (claude, codex)
        });
        let mut groups = vec![claude, codex];
        groups.extend(read_kimi_limits(&home));
        groups
    })
    .await
    .map_err(|error| error.to_string())?;

    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some(Cached {
            at: Instant::now(),
            groups: groups.clone(),
        });
    }
    Ok(groups)
}

#[cfg(test)]
#[path = "provider_limits_tests.rs"]
mod tests;
