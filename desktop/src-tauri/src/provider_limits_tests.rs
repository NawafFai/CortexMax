use super::*;

/// Pin a file's modification time, so "newest" is unambiguous in a test that
/// writes both files in the same instant.
fn set_mtime(path: &Path, epoch_seconds: u64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(UNIX_EPOCH + Duration::from_secs(epoch_seconds))
        .unwrap();
}

/// An instant before every fixture's reset time, so a reading is current.
const BEFORE_RESET: i64 = 1_790_000_000;

/// The exact shape the installed CLI prints. Parsing must survive the prose
/// around it, because the report is written for a terminal, not for us.
const CLAUDE_REPORT: &str = "\
You are currently using your subscription to power your Claude Code usage

Current session: 33% used · resets Sep 19 at 4pm (Europe/Istanbul)
Current week (all models): 67% used · resets Sep 23 at 9am (Europe/Istanbul)
Current week (Fable): 100% used · resets Sep 23 at 9am (Europe/Istanbul)

What's contributing to your limits usage?
Last 24h · 1153 requests · 21 sessions
  86% of your usage was at >150k context
";

#[test]
fn the_three_claude_windows_are_read_with_their_reset_times() {
    let windows = parse_claude_usage(CLAUDE_REPORT);
    assert_eq!(windows.len(), 3, "{windows:#?}");

    assert_eq!(windows[0].label, "Session");
    assert_eq!(windows[0].used_percent, 33.0);
    assert_eq!(
        windows[0].resets.as_deref(),
        Some("Sep 19 at 4pm (Europe/Istanbul)"),
        "the CLI already rendered the owner's timezone; keep its wording"
    );

    assert_eq!(windows[1].label, "Week · all models");
    assert_eq!(windows[1].used_percent, 67.0);
    assert_eq!(windows[2].label, "Week · Fable");
    assert_eq!(windows[2].used_percent, 100.0);
}

/// The prose lines also contain percentages ("86% of your usage was at…").
/// Reading one as a limit would invent a window the owner does not have.
#[test]
fn percentages_in_the_prose_are_not_mistaken_for_limits() {
    let windows = parse_claude_usage(CLAUDE_REPORT);
    assert!(
        windows
            .iter()
            .all(|w| w.label.starts_with("Session") || w.label.starts_with("Week")),
        "a prose line leaked in: {windows:#?}"
    );
}

/// A model-specific window is named by the CLI, not by us — a new model must
/// appear under its own name without a code change.
#[test]
fn an_unknown_model_window_keeps_the_name_the_cli_gave_it() {
    let windows = parse_claude_usage("Current week (Someday-9): 4% used · resets tomorrow");
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].label, "Week · Someday-9");
    assert_eq!(windows[0].resets.as_deref(), Some("tomorrow"));
}

/// A line with no reset clause is still a real reading; the window must not be
/// dropped just because the CLI omitted the time.
#[test]
fn a_window_without_a_reset_time_is_still_reported() {
    let windows = parse_claude_usage("Current session: 12% used");
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].used_percent, 12.0);
    assert_eq!(windows[0].resets, None);
}

/// Output that says nothing about limits must produce no windows, so the UI
/// says "unavailable" instead of drawing empty bars.
#[test]
fn unrecognized_output_reports_nothing_rather_than_zero() {
    assert!(parse_claude_usage("Please run /login first").is_empty());
    assert!(parse_claude_usage("").is_empty());
}

/// An unreadable runtime must never render as 0% — on a bar that is
/// indistinguishable from "nothing used", which is the opposite claim.
#[test]
fn an_unavailable_group_carries_a_reason_and_no_windows() {
    let group = LimitGroup::unavailable("claude", "Claude", "could not run the claude CLI");
    assert!(group.windows.is_empty());
    assert_eq!(
        group.unavailable.as_deref(),
        Some("could not run the claude CLI")
    );
}

/// The real payload Codex writes into its rollouts.
fn codex_limits() -> serde_json::Value {
    serde_json::json!({
        "limit_id": "codex",
        "limit_name": null,
        "primary": { "used_percent": 99.0, "window_minutes": 10080, "resets_at": 1790157201 },
        "secondary": null,
        "credits": { "has_credits": false, "unlimited": false, "balance": "0" },
        "plan_type": "prolite"
    })
}

#[test]
fn the_codex_weekly_window_is_read_from_its_own_report() {
    let windows = parse_codex_rate_limits(&codex_limits(), BEFORE_RESET);
    assert_eq!(windows.len(), 1, "secondary is null and must be skipped");
    assert_eq!(windows[0].label, "Week");
    assert_eq!(windows[0].used_percent, 99.0);
    assert_eq!(windows[0].resets_at, Some(1790157201));
}

/// Codex reports both windows when a plan has them; neither may be dropped.
#[test]
fn both_codex_windows_are_read_when_present() {
    let value = serde_json::json!({
        "primary": { "used_percent": 12.5, "window_minutes": 300, "resets_at": 1790157201 },
        "secondary": { "used_percent": 80.0, "window_minutes": 10080, "resets_at": 1790157999 },
    });
    let windows = parse_codex_rate_limits(&value, BEFORE_RESET);
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].label, "5 hours");
    assert_eq!(windows[0].used_percent, 12.5);
    assert_eq!(windows[1].label, "Week");
}

/// Codex states a window in minutes; the label is derived, so a window length
/// nobody has seen before still reads as a duration rather than a number.
#[test]
fn window_lengths_are_named_by_duration() {
    assert_eq!(codex_window_label(10_080), "Week");
    assert_eq!(codex_window_label(20_160), "2 weeks");
    assert_eq!(codex_window_label(1_440), "Day");
    assert_eq!(codex_window_label(300), "5 hours");
    assert_eq!(codex_window_label(45), "45 minutes");
}

/// A malformed entry is skipped rather than defaulted, so a missing percentage
/// never renders as an empty bar that reads as "nothing used".
#[test]
fn a_window_without_a_percentage_is_skipped() {
    let value = serde_json::json!({
        "primary": { "window_minutes": 10080, "resets_at": 1790157201 },
        "secondary": { "used_percent": 5.0, "window_minutes": 300 },
    });
    let windows = parse_codex_rate_limits(&value, BEFORE_RESET);
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].used_percent, 5.0);
    assert_eq!(
        windows[0].resets, None,
        "no resets_at means no reset text, not an empty string"
    );
}

/// Percentages are clamped: a provider reporting 103% must not overflow a bar.
#[test]
fn percentages_are_clamped_to_the_bar() {
    let value = serde_json::json!({
        "primary": { "used_percent": 103.0, "window_minutes": 10080 },
    });
    assert_eq!(
        parse_codex_rate_limits(&value, BEFORE_RESET)[0].used_percent,
        100.0
    );
    assert_eq!(
        parse_claude_usage("Current session: 250% used")[0].used_percent,
        100.0
    );
}

/// The wire shape is what the UI reads; a rename silently empties the panel.
#[test]
fn the_wire_shape_matches_what_the_ui_reads() {
    let group = LimitGroup {
        runtime: "codex".to_string(),
        label: "Codex".to_string(),
        windows: parse_codex_rate_limits(&codex_limits(), BEFORE_RESET),
        plan: Some("prolite".to_string()),
        note: Some("credits: 0".to_string()),
        unavailable: None,
        age_seconds: Some(42),
    };
    let json = serde_json::to_string(&group).unwrap();
    assert!(json.contains("\"usedPercent\":99.0"), "{json}");
    assert!(json.contains("\"resetsAt\":1790157201"), "{json}");
    assert!(json.contains("\"ageSeconds\":42"), "{json}");
    assert!(json.contains("\"stale\":false"), "{json}");
    assert!(!json.contains("used_percent"), "{json}");
}

/// Reading the newest rollout is the whole mechanism for Codex; picking the
/// wrong file reports a stale limit as current.
#[test]
fn the_newest_rollout_is_the_one_read() {
    let dir = tempfile::tempdir().unwrap();
    let day = dir.path().join("2026/09/17");
    std::fs::create_dir_all(&day).unwrap();

    // A long-running session keeps being appended to after a later one starts,
    // so the fresher reading is the newest by MTIME, not by name. The stale
    // file is named so that it sorts last: a name-ordered implementation picks
    // it and fails here.
    let stale = day.join("rollout-2026-09-17T23-00-00-zzz.jsonl");
    let fresh = day.join("rollout-2026-09-17T08-00-00-aaa.jsonl");
    std::fs::write(&stale, "{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":10.0,\"window_minutes\":10080}}}}\n").unwrap();
    std::fs::write(&fresh, "{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":88.0,\"window_minutes\":10080}}}}\n").unwrap();
    set_mtime(&stale, 1_790_000_000);
    set_mtime(&fresh, 1_790_100_000);

    let found = codex_rollouts_newest_first(dir.path())
        .into_iter()
        .next()
        .expect("a rollout");
    let limits = last_rate_limits_in(&found).expect("limits");
    assert_eq!(
        parse_codex_rate_limits(&limits, BEFORE_RESET)[0].used_percent,
        88.0,
        "a stale rollout that merely sorts later by name was read as current"
    );
}

/// A rollout records a limit after every turn; the last one is the current
/// reading, and an earlier line must not win.
#[test]
fn the_last_reading_in_a_rollout_wins() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rollout.jsonl");
    std::fs::write(
        &file,
        "{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":20.0,\"window_minutes\":10080}}}}\n\
         {\"payload\":{\"type\":\"token_count\"}}\n\
         {\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":61.0,\"window_minutes\":10080}}}}\n",
    )
    .unwrap();

    let limits = last_rate_limits_in(&file).expect("limits");
    assert_eq!(
        parse_codex_rate_limits(&limits, BEFORE_RESET)[0].used_percent,
        61.0
    );
}

/// Codex writes `"rate_limits": null` on turns that carried no reading; those
/// lines must be stepped over rather than ending the search.
#[test]
fn a_null_reading_is_skipped_for_the_last_real_one() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rollout.jsonl");
    std::fs::write(
        &file,
        "{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":61.0,\"window_minutes\":10080}}}}\n\
         {\"payload\":{\"rate_limits\":null}}\n",
    )
    .unwrap();

    let limits = last_rate_limits_in(&file).expect("the null line must not end the search");
    assert_eq!(
        parse_codex_rate_limits(&limits, BEFORE_RESET)[0].used_percent,
        61.0
    );
}

/// A machine that has never run Codex is a normal state, not an error.
#[test]
fn a_missing_codex_directory_reports_why_rather_than_failing() {
    let dir = tempfile::tempdir().unwrap();
    let group = read_codex_limits(dir.path());
    assert!(group.windows.is_empty());
    assert!(group.unavailable.is_some());
    assert_eq!(group.runtime, "codex");
}

/// Codex only writes a limit when it takes a turn. If its window rolls over
/// while Codex is idle, the last reading it wrote is all there is — and
/// showing that stale 99% as current would tell an owner they are still locked
/// out when the limit has in fact reset.
#[test]
fn a_codex_window_past_its_reset_time_is_marked_stale() {
    let after_reset = 1_790_157_202;
    let windows = parse_codex_rate_limits(&codex_limits(), after_reset);
    assert_eq!(windows.len(), 1);
    assert!(
        windows[0].stale,
        "a reading for a window that has already reset must not read as current"
    );
    assert_eq!(
        windows[0].used_percent, 99.0,
        "the figure is kept so the UI can say what it was, not silently zeroed"
    );
}

/// The boundary belongs to the new window: at exactly the reset instant the
/// old reading no longer describes anything.
#[test]
fn the_reset_instant_itself_counts_as_reset() {
    let windows = parse_codex_rate_limits(&codex_limits(), 1_790_157_201);
    assert!(windows[0].stale);
    assert!(!parse_codex_rate_limits(&codex_limits(), 1_790_157_200)[0].stale);
}

/// A window the provider gave no reset time for cannot be proven stale, so it
/// must not be guessed at either way.
#[test]
fn a_window_without_a_reset_time_is_never_marked_stale() {
    let value = serde_json::json!({
        "primary": { "used_percent": 40.0, "window_minutes": 10080 },
    });
    assert!(!parse_codex_rate_limits(&value, i64::MAX)[0].stale);
}

/// Claude is read live on every fetch, so its windows are never stale.
#[test]
fn claude_windows_are_never_stale() {
    assert!(parse_claude_usage(CLAUDE_REPORT).iter().all(|w| !w.stale));
}

// ---------------------------------------------------------------------------
// Reset-clause parsing, for the live countdown
// ---------------------------------------------------------------------------

use chrono::NaiveDate;

fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap()
}

/// The exact clauses the CLI writes must read correctly, or the countdown
/// counts to the wrong instant.
#[test]
fn the_cli_reset_clauses_parse() {
    assert_eq!(
        parse_reset_parts("Sep 19 at 4pm (Europe/Istanbul)"),
        Some((9, 19, 16, 0))
    );
    assert_eq!(
        parse_reset_parts("Sep 23 at 8:59am (Europe/Istanbul)"),
        Some((9, 23, 8, 59))
    );
    assert_eq!(parse_reset_parts("Jan 2 at 9am"), Some((1, 2, 9, 0)));
}

/// Noon and midnight do not follow the +12 rule, and getting them wrong puts
/// the countdown twelve hours out.
#[test]
fn noon_and_midnight_are_not_shifted_by_twelve() {
    assert_eq!(parse_reset_parts("Sep 23 at 12pm"), Some((9, 23, 12, 0)));
    assert_eq!(parse_reset_parts("Sep 23 at 12am"), Some((9, 23, 0, 0)));
    assert_eq!(parse_reset_parts("Sep 23 at 1pm"), Some((9, 23, 13, 0)));
    assert_eq!(parse_reset_parts("Sep 23 at 1am"), Some((9, 23, 1, 0)));
}

/// Prose this does not recognize yields no instant, so the UI shows the CLI's
/// own wording and no countdown — better than counting down to a guess.
#[test]
fn unreadable_prose_yields_no_instant() {
    for text in [
        "tomorrow",
        "",
        "Sep at 4pm",
        "Smarch 19 at 4pm",
        "Sep 19 at 4",
    ] {
        assert_eq!(parse_reset_parts(text), None, "{text} should not parse");
    }
}

/// An hour outside a 12-hour clock is not a time this format can express.
#[test]
fn an_impossible_clock_is_refused() {
    assert_eq!(parse_reset_parts("Sep 19 at 13pm"), None);
    assert_eq!(parse_reset_parts("Sep 19 at 0am"), None);
    assert_eq!(parse_reset_parts("Sep 19 at 4:70pm"), None);
}

/// The CLI omits the year, so a reset on Jan 2 read on Dec 30 means next
/// January — not the one ten months gone.
#[test]
fn a_reset_in_january_read_in_december_rolls_to_next_year() {
    let resolved = nearest_future((1, 2, 9, 0), at(2026, 12, 30, 18, 0)).unwrap();
    assert_eq!(resolved, at(2027, 1, 2, 9, 0));
}

/// The ordinary case stays in this year.
#[test]
fn a_reset_later_this_year_stays_in_this_year() {
    let resolved = nearest_future((9, 23, 8, 59), at(2026, 9, 19, 15, 0)).unwrap();
    assert_eq!(resolved, at(2026, 9, 23, 8, 59));
}

/// A reset a moment ago is due now, not a year away. Rolling it forward would
/// turn "resets any second" into "resets in 12 months".
#[test]
fn a_reset_that_just_passed_is_not_thrown_a_year_forward() {
    let resolved = nearest_future((9, 19, 16, 0), at(2026, 9, 19, 16, 0)).unwrap();
    assert_eq!(resolved, at(2026, 9, 19, 16, 0));

    let just_after = nearest_future((9, 19, 16, 0), at(2026, 9, 19, 16, 0)).unwrap();
    assert_eq!(just_after, at(2026, 9, 19, 16, 0));
}

/// A reset well in the past belongs to next year's occurrence.
#[test]
fn a_reset_long_past_rolls_to_next_year() {
    let resolved = nearest_future((1, 5, 9, 0), at(2026, 9, 19, 12, 0)).unwrap();
    assert_eq!(resolved, at(2027, 1, 5, 9, 0));
}

/// Feb 29 in a common year is not a date; the next candidate year is tried
/// rather than failing the whole countdown.
#[test]
fn a_leap_day_falls_through_to_a_year_that_has_one() {
    let resolved = nearest_future((2, 29, 9, 0), at(2027, 6, 1, 12, 0));
    assert_eq!(
        resolved,
        Some(at(2028, 2, 29, 9, 0)),
        "2027 has no Feb 29, so the next candidate year must be used"
    );
}

/// Kimi is a first-class runtime here but publishes no limit this can read.
/// It is still listed with that reason: a runtime the owner is actively
/// running must not simply be absent from the panel that says how much room is
/// left, because absence reads as "not installed" — a different, wrong claim.
#[test]
fn an_installed_kimi_is_listed_with_the_reason_it_has_no_limit() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".kimi-code/bin")).unwrap();
    std::fs::write(dir.path().join(".kimi-code/bin/kimi"), b"#!/bin/sh\n").unwrap();

    let group = read_kimi_limits(dir.path()).expect("an installed kimi is listed");
    assert_eq!(group.runtime, "kimi");
    assert!(group.windows.is_empty(), "it has no windows to show");
    let reason = group.unavailable.as_deref().expect("a stated reason");
    // Verified on a real install: `kimi -p "/usage"` answers with a
    // description of the command rather than the figures, and nothing under
    // `~/.kimi-code` caches them. So the reason has to point at the one place
    // they do exist — the interactive CLI — and name the command that shows
    // them, or the owner is told "unavailable" with nowhere to go.
    assert!(
        reason.contains("interactive"),
        "the reason must say where the figure actually lives: {reason}"
    );
    assert!(
        reason.contains("/usage"),
        "and name the command that shows it: {reason}"
    );
    assert!(
        reason.contains("daily limit"),
        "a dead end is not enough; it must say what the owner can do: {reason}"
    );
}

/// On a machine without Kimi the note would be noise about a runtime the owner
/// does not use.
#[test]
fn a_machine_without_kimi_says_nothing_about_it() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_kimi_limits(dir.path()).is_none());
}

// ---------------------------------------------------------------------------
// Codex with nothing left to spend
// ---------------------------------------------------------------------------

/// The shape Codex actually writes once the account runs dry: both rate-limit
/// slots null, because there is no consumption rate to describe when nothing
/// can be consumed.
fn spent_codex_limits() -> serde_json::Value {
    serde_json::json!({
        "limit_id": "premium",
        "primary": serde_json::Value::Null,
        "secondary": serde_json::Value::Null,
        "credits": { "has_credits": false, "unlimited": false, "balance": "0" },
        "plan_type": "prolite",
    })
}

/// An exhausted provider is the one an owner most needs on screen: it is the
/// reason the agents on it stopped. A zero balance is a figure that was read,
/// not guessed, so it earns a bar rather than an omission.
#[test]
fn codex_with_no_credit_still_reports_a_full_bar() {
    let window = codex_credit_window(&spent_codex_limits()).expect("a spent window");
    assert_eq!(window.used_percent, 100.0);
    // What ran out is the plan's allowance; the zero balance is only the
    // add-on credit that would have carried past it.
    assert_eq!(window.label, "Usage");
    // The spent reading carries no reset of its own; `read_codex_limits`
    // finds one elsewhere.
    assert_eq!(window.resets_at, None);
    assert!(!window.stale);
}

/// Unlimited credit has no ceiling, so a bar would be drawing a limit that
/// does not exist.
#[test]
fn unlimited_codex_credit_draws_no_bar() {
    let value = serde_json::json!({
        "credits": { "has_credits": true, "unlimited": true, "balance": "0" },
    });
    assert!(codex_credit_window(&value).is_none());
}

/// Holding a balance is not the same as having been measured. An account with
/// credit and no window has an unknown position, and a guess would be worse
/// than the omission.
#[test]
fn codex_with_credit_left_and_no_window_stays_silent() {
    let value = serde_json::json!({
        "credits": { "has_credits": true, "unlimited": false, "balance": "42.00" },
    });
    assert!(codex_credit_window(&value).is_none());
}

/// A rollout with no credits object at all says nothing about credit.
#[test]
fn codex_without_a_credits_object_draws_no_bar() {
    let value = serde_json::json!({ "plan_type": "prolite" });
    assert!(codex_credit_window(&value).is_none());
}

/// End to end, because the two halves only matter together: an exhausted
/// account has to reach the panel as a bar, and the note has to carry the
/// words `agent_leave` reads when it tells the owner why an agent stopped.
#[test]
fn a_spent_codex_account_reaches_the_panel_as_a_full_bar() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join(".codex/sessions/2026/09/20");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(
        day.join("rollout-2026-09-20T16-00-00-aaa.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({ "payload": { "rate_limits": spent_codex_limits() } })
        ),
    )
    .unwrap();

    let group = read_codex_limits(home.path());
    assert!(
        group.unavailable.is_none(),
        "a read balance is a figure, not an unknown: {:?}",
        group.unavailable
    );
    assert_eq!(group.windows.len(), 1);
    assert_eq!(group.windows[0].used_percent, 100.0);
    assert_eq!(group.plan.as_deref(), Some("prolite"));

    let note = group.note.expect("a note saying why");
    assert!(note.contains("no credit"), "agent_leave reads this: {note}");
    assert!(
        note.contains('0'),
        "and the owner should see the balance itself: {note}"
    );
}

/// The credit bar is the fallback for having no window, not an extra row
/// beside one. A provider reporting a real window says all it needs to.
#[test]
fn a_codex_account_with_a_real_window_shows_only_that_window() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join(".codex/sessions/2026/09/20");
    std::fs::create_dir_all(&day).unwrap();
    // `codex_limits` carries a real weekly window *and* a zero balance.
    std::fs::write(
        day.join("rollout.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({ "payload": { "rate_limits": codex_limits() } })
        ),
    )
    .unwrap();

    let group = read_codex_limits(home.path());
    assert_eq!(group.windows.len(), 1);
    assert_eq!(group.windows[0].used_percent, 99.0);
    assert_ne!(group.windows[0].label, "Usage");
    // The zero balance is an add-on never bought, not the reason anything
    // stopped: `agent_leave` reads "no credit" as "nothing to wait for".
    let note = group.note.unwrap_or_default();
    assert!(!note.contains("no credit"), "{note}");
}

/// A Codex session that was opened and never used writes no `rate_limits`
/// line. Stopping at it is how a provider that *did* report vanishes from the
/// panel; the reading is one file back, and it is still the current one.
#[test]
fn a_silent_newest_rollout_does_not_hide_the_reading_behind_it() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join(".codex/sessions/2026/09/20");
    std::fs::create_dir_all(&day).unwrap();

    let reported = day.join("rollout-2026-09-20T15-00-00-aaa.jsonl");
    let silent = day.join("rollout-2026-09-20T16-00-00-bbb.jsonl");
    std::fs::write(
        &reported,
        format!(
            "{}\n",
            serde_json::json!({ "payload": { "rate_limits": spent_codex_limits() } })
        ),
    )
    .unwrap();
    // Opened, wrote a turn, never made a request.
    std::fs::write(&silent, "{\"payload\":{\"type\":\"session_meta\"}}\n").unwrap();
    set_mtime(&reported, 1_790_000_000);
    set_mtime(&silent, 1_790_100_000);

    let group = read_codex_limits(home.path());
    assert!(
        group.unavailable.is_none(),
        "the reading one file back is still the current one: {:?}",
        group.unavailable
    );
    assert_eq!(group.windows.len(), 1);
    assert_eq!(group.windows[0].used_percent, 100.0);
}

/// The walk back is bounded, and a genuinely silent history still says so
/// rather than reporting a figure it never read.
#[test]
fn a_history_with_no_reading_at_all_still_reports_why() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join(".codex/sessions/2026/09/20");
    std::fs::create_dir_all(&day).unwrap();
    for i in 0..3 {
        std::fs::write(
            day.join(format!("rollout-2026-09-20T0{i}-00-00-x.jsonl")),
            "{\"payload\":{\"type\":\"session_meta\"}}\n",
        )
        .unwrap();
    }
    let group = read_codex_limits(home.path());
    let reason = group.unavailable.expect("a reason");
    assert!(reason.contains("not reported a limit yet"), "{reason}");
}

/// An exhausted account reports no reset in its latest reading, but the
/// week's earlier readings still carry one. That is the time the owner needs
/// — when Codex comes back — and it is Codex's own figure, not a guess.
#[test]
fn a_spent_codex_account_shows_when_it_comes_back() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join(".codex/sessions/2026/09/20");
    std::fs::create_dir_all(&day).unwrap();
    let earlier = day.join("rollout-2026-09-20T10-00-00-aaa.jsonl");
    let latest = day.join("rollout-2026-09-20T16-00-00-bbb.jsonl");
    // Earlier in the week: a real window with a reset time far in the future.
    std::fs::write(
        &earlier,
        "{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":89.0,\"window_minutes\":10080,\"resets_at\":4102444800}}}}\n",
    )
    .unwrap();
    // Latest: spent, so the slots are null and no reset is recorded.
    std::fs::write(
        &latest,
        format!(
            "{}\n",
            serde_json::json!({ "payload": { "rate_limits": spent_codex_limits() } })
        ),
    )
    .unwrap();
    set_mtime(&earlier, 1_790_000_000);
    set_mtime(&latest, 1_790_100_000);

    let group = read_codex_limits(home.path());
    assert_eq!(group.windows.len(), 1);
    assert_eq!(group.windows[0].used_percent, 100.0);
    assert_eq!(
        group.windows[0].resets_at,
        Some(4_102_444_800),
        "the reset from earlier in the week should reach the credit bar"
    );
}

/// A reset that has already passed is not when anything comes back.
#[test]
fn a_past_reset_is_not_offered_as_the_return_time() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join(".codex/sessions/2026/09/20");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(
        day.join("rollout-2026-09-20T10-00-00-aaa.jsonl"),
        "{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":89.0,\"window_minutes\":10080,\"resets_at\":1000}}}}\n",
    )
    .unwrap();
    std::fs::write(
        day.join("rollout-2026-09-20T16-00-00-bbb.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({ "payload": { "rate_limits": spent_codex_limits() } })
        ),
    )
    .unwrap();
    let group = read_codex_limits(home.path());
    assert_eq!(group.windows[0].resets_at, None);
}

/// A spent reading followed by the reset that ends it: the account is back.
///
/// The morning this was found, Codex refused a turn at 11:34 with "try again
/// at 12:53", and the panel still read "no credit left" all afternoon — the
/// exhaustion had no way to end except a new turn, and nothing was running.
#[test]
fn a_spent_account_is_back_once_the_reset_after_it_passes() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join(".codex/sessions/2026/09/23");
    std::fs::create_dir_all(&day).unwrap();
    let week = day.join("rollout-2026-09-17T10-00-00-aaa.jsonl");
    let spent = day.join("rollout-2026-09-23T08-34-00-bbb.jsonl");
    // Read on the 17th: the weekly window resets at 1_790_157_180.
    std::fs::write(
        &week,
        "{\"timestamp\":\"2026-09-17T15:04:50Z\",\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":95.0,\"window_minutes\":10080,\"resets_at\":1790157180}}}}\n",
    )
    .unwrap();
    // Read spent at 08:34, before that reset.
    std::fs::write(
        &spent,
        format!(
            "{}\n",
            serde_json::json!({
                "timestamp": "2026-09-23T08:34:28Z",
                "payload": { "rate_limits": spent_codex_limits() },
            })
        ),
    )
    .unwrap();
    set_mtime(&week, 1_790_000_000);
    set_mtime(&spent, 1_790_152_468);

    let group = read_codex_limits(home.path());
    assert_eq!(group.windows.len(), 1);
    let window = &group.windows[0];
    assert_eq!(window.resets_at, Some(1_790_157_180));
    assert!(window.stale, "the reset after the exhaustion has passed");
    assert_eq!(
        group.note, None,
        "an account past its reset is not out of credit"
    );
}

/// A reset from before the account ran dry describes a window that was
/// already over; it cannot be what brings the account back.
#[test]
fn a_reset_from_before_the_account_ran_dry_does_not_end_it() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join(".codex/sessions/2026/09/23");
    std::fs::create_dir_all(&day).unwrap();
    let earlier = day.join("rollout-2026-09-10T10-00-00-aaa.jsonl");
    let spent = day.join("rollout-2026-09-23T08-34-00-bbb.jsonl");
    std::fs::write(
        &earlier,
        "{\"timestamp\":\"2026-09-10T10:00:00Z\",\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":40.0,\"window_minutes\":10080,\"resets_at\":1789700000}}}}\n",
    )
    .unwrap();
    std::fs::write(
        &spent,
        format!(
            "{}\n",
            serde_json::json!({
                "timestamp": "2026-09-23T08:34:28Z",
                "payload": { "rate_limits": spent_codex_limits() },
            })
        ),
    )
    .unwrap();
    set_mtime(&earlier, 1_789_600_000);
    set_mtime(&spent, 1_790_152_468);

    let group = read_codex_limits(home.path());
    let window = &group.windows[0];
    assert_eq!(window.resets_at, None);
    assert!(!window.stale);
    let note = group.note.expect("still spent");
    assert!(note.contains("no credit"), "{note}");
}

// ---------------------------------------------------------------------------
// Codex asked live
// ---------------------------------------------------------------------------

/// What Codex's app-server answered to `account/rateLimits/read` the afternoon
/// this was written: a week that had just reset, no add-on credit bought, and
/// one free reset granted.
fn live_answer() -> serde_json::Value {
    serde_json::json!({
        "ordinaryUsageAllowed": true,
        "rateLimits": {
            "limitId": "codex",
            "primary": { "usedPercent": 0, "windowDurationMins": 10080, "resetsAt": 4_102_444_800_i64 },
            "secondary": null,
            "credits": { "hasCredits": false, "unlimited": false, "balance": 0 },
            "spendControlReached": false,
            "planType": "prolite",
        },
        "rateLimitResetCredits": { "availableCount": 1 },
    })
}

/// The owner saw "reset · awaiting next turn" where the provider's own page
/// said "100% left, resets Sep 30". Asked live, the panel says the same.
#[test]
fn a_live_answer_reads_as_the_week_it_describes() {
    let group = codex_group_from_live(&live_answer(), 1_790_157_300).expect("a group");
    assert!(group.unavailable.is_none());
    assert_eq!(group.windows.len(), 1);
    let week = &group.windows[0];
    assert_eq!(week.label, "Week");
    assert_eq!(week.used_percent, 0.0);
    assert_eq!(week.window_minutes, Some(10_080));
    assert_eq!(week.resets_at, Some(4_102_444_800));
    assert!(week.resets.is_some(), "the reset is stated, not left blank");
    assert!(!week.stale);
    assert_eq!(group.plan.as_deref(), Some("prolite"));
    assert_eq!(
        group.age_seconds, None,
        "asked just now, not read from a file"
    );
}

/// A full week with no add-on credit is not an account with "no credit".
/// `agent_leave` reads that phrase as "nothing to wait for", so saying it here
/// would send every Codex agent on leave with the whole week in hand.
#[test]
fn a_fresh_week_without_add_on_credit_is_not_no_credit() {
    let group = codex_group_from_live(&live_answer(), 1_790_157_300).unwrap();
    let note = group.note.unwrap_or_default();
    assert!(!note.to_ascii_lowercase().contains("no credit"), "{note}");
}

#[test]
fn a_free_reset_codex_granted_is_shown_beside_the_bar() {
    let group = codex_group_from_live(&live_answer(), 1_790_157_300).unwrap();
    assert_eq!(group.note.as_deref(), Some("1 free reset available"));

    let mut answer = live_answer();
    answer["rateLimitResetCredits"]["availableCount"] = serde_json::json!(0);
    let group = codex_group_from_live(&answer, 1_790_157_300).unwrap();
    assert_eq!(group.note, None, "none left is nothing to say");
}

/// A spent week says when it comes back, so the window carries the reason and
/// "no credit" — which has no return date — stays out of the note.
#[test]
fn a_spent_live_week_carries_its_own_return_time() {
    let mut answer = live_answer();
    answer["rateLimits"]["primary"]["usedPercent"] = serde_json::json!(100);
    let group = codex_group_from_live(&answer, 1_790_157_300).unwrap();
    assert_eq!(group.windows[0].used_percent, 100.0);
    assert_eq!(group.windows[0].resets_at, Some(4_102_444_800));
    let note = group.note.unwrap_or_default();
    assert!(!note.contains("no credit"), "{note}");
}

/// The shape Codex gives once the plan is gone: no window, zero balance.
#[test]
fn a_spent_live_account_with_no_window_still_draws_a_full_bar() {
    let answer = serde_json::json!({
        "rateLimits": {
            "primary": null,
            "secondary": null,
            "credits": { "hasCredits": false, "unlimited": false, "balance": 0 },
            "planType": "prolite",
        },
    });
    let group = codex_group_from_live(&answer, 1_790_157_300).expect("a group");
    assert_eq!(group.windows.len(), 1);
    assert_eq!(group.windows[0].used_percent, 100.0);
    let note = group.note.expect("a reason");
    assert!(note.contains("no credit"), "agent_leave reads this: {note}");
    assert!(
        note.contains('0'),
        "the balance itself, as a figure: {note}"
    );
}

/// Nothing to draw hands over to the session log rather than an empty panel.
#[test]
fn a_live_answer_with_nothing_to_draw_leaves_it_to_the_session_log() {
    for answer in [
        serde_json::json!({ "rateLimits": null }),
        serde_json::json!({}),
        serde_json::json!({ "rateLimits": { "primary": null, "secondary": null } }),
    ] {
        assert!(
            codex_group_from_live(&answer, 1_790_157_300).is_none(),
            "{answer}"
        );
    }
}

/// One parser reads both spellings, so the live answer and the same reading
/// logged by a turn must draw the identical bar.
#[test]
fn a_live_answer_and_the_same_reading_logged_draw_the_same_bar() {
    let live = serde_json::json!({
        "rateLimits": {
            "primary": { "usedPercent": 99.0, "windowDurationMins": 10080, "resetsAt": 1_790_157_201 },
            "secondary": null,
            "credits": { "hasCredits": false, "unlimited": false, "balance": 0 },
            "planType": "prolite",
        },
    });
    let from_live = codex_group_from_live(&live, 1_790_000_000).unwrap();
    assert_eq!(
        from_live.windows,
        parse_codex_rate_limits(&codex_limits(), 1_790_000_000)
    );
}

/// A stand-in for `codex app-server`: a shell script speaking its protocol.
#[cfg(unix)]
fn fake_codex(dir: &Path, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("codex");
    std::fs::write(&path, format!("#!/bin/sh\n{script}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// The question is asked only after the handshake, the notification Codex
/// sends first is skipped, and the answer to the question is what comes back.
#[cfg(unix)]
#[test]
fn codex_is_asked_after_the_handshake_and_its_answer_returned() {
    let dir = tempfile::tempdir().unwrap();
    let codex = fake_codex(
        dir.path(),
        r#"ready=
while IFS= read -r line; do
  case "$line" in
    *'"initialize"'*) echo '{"id":0,"result":{}}' ;;
    *'"initialized"'*) ready=1 ;;
    *'account/rateLimits/read'*)
      if [ -z "$ready" ]; then echo '{"id":1,"error":{"message":"not initialized"}}'; continue; fi
      echo '{"method":"account/rateLimits/updated","params":{}}'
      echo '{"id":1,"result":{"rateLimits":{"primary":{"usedPercent":12,"windowDurationMins":10080,"resetsAt":4102444800}}}}' ;;
  esac
done
"#,
    );
    let answer = ask_codex_for_limits(&codex, Duration::from_secs(10)).expect("an answer");
    assert_eq!(answer["rateLimits"]["primary"]["usedPercent"], 12);
}

/// A refusal is reported as one, never read as a set of limits.
#[cfg(unix)]
#[test]
fn an_error_from_codex_is_reported_rather_than_read_as_limits() {
    let dir = tempfile::tempdir().unwrap();
    let codex = fake_codex(
        dir.path(),
        r#"while IFS= read -r line; do
  case "$line" in
    *'"initialize"'*) echo '{"id":0,"result":{}}' ;;
    *'account/rateLimits/read'*) echo '{"id":1,"error":{"code":-32600,"message":"not signed in"}}' ;;
  esac
done
"#,
    );
    let error = ask_codex_for_limits(&codex, Duration::from_secs(10)).unwrap_err();
    assert!(error.contains("not signed in"), "{error}");
}

/// A server that never answers costs the panel the timeout, not a hang.
#[cfg(unix)]
#[test]
fn a_codex_that_never_answers_is_given_up_on() {
    let dir = tempfile::tempdir().unwrap();
    // Reads forever and never writes, with its output still open: a server
    // that is alive but silent, not one that has gone away.
    let codex = fake_codex(dir.path(), "while IFS= read -r line; do :; done\n");
    let started = Instant::now();
    let error = ask_codex_for_limits(&codex, Duration::from_millis(500)).unwrap_err();
    assert!(error.contains("in time"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(unix)]
#[test]
fn a_codex_that_exits_is_reported_rather_than_waited_on() {
    let dir = tempfile::tempdir().unwrap();
    let codex = fake_codex(dir.path(), "exit 0\n");
    let started = Instant::now();
    let error = ask_codex_for_limits(&codex, Duration::from_secs(10)).unwrap_err();
    assert!(
        error.contains("exited") || error.contains("could not write"),
        "{error}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// The adapter's vendored binary is found whatever platform package npm chose.
#[test]
fn the_vendored_codex_is_found_under_its_platform_package() {
    let scope = tempfile::tempdir().unwrap();
    assert_eq!(vendored_codex_in(scope.path()), None);
    let bin = scope
        .path()
        .join("codex-linux-x64/vendor/x86_64-unknown-linux-musl/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    std::fs::write(bin.join(name), "").unwrap();
    // A sibling package that is not a Codex platform build is ignored.
    std::fs::create_dir_all(scope.path().join("other/vendor/x/bin")).unwrap();
    assert_eq!(vendored_codex_in(scope.path()), Some(bin.join(name)));
}
