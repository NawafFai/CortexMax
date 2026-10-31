//! Minutes of a meeting, written the moment it ends.
//!
//! In a meeting the owner's speech is transcribed into the meeting's own
//! channel and each agent answers there. Ending the meeting archives that
//! channel — and with it the only record of what the owner asked for. So just
//! before the archive the conversation is read once, then written up twice:
//! in full on this Mac under `meeting-notes/`, and as the owner's requests
//! posted into the channel the meeting was held from, addressed to the lead
//! agent who attended, so what was said becomes work instead of vanishing.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{FixedOffset, Local, Offset, TimeZone};
use tauri::Manager;

use crate::app_state::AppState;
use crate::events;
use crate::relay::{query_relay, submit_event};

/// The agent the minutes are addressed to, when it attended.
const LEAD_AGENT: &str = "CORTEX";
/// Longest run of the owner's words posted into the channel; the file keeps all.
const MAX_POSTED_CHARS: usize = 6_000;
/// Most messages read from one meeting.
const MAX_MESSAGES: usize = 2_000;
/// How long reading the meeting may hold up ending it.
const READ_TIMEOUT: Duration = Duration::from_secs(6);
/// How long looking up names may take once the meeting is over.
const NAMES_TIMEOUT: Duration = Duration::from_secs(8);

/// One thing said in the meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Utterance {
    /// Unix seconds.
    pub at: i64,
    /// Speaker's pubkey (hex).
    pub speaker: String,
    pub text: String,
}

/// What the meeting was and who was in it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Meeting {
    pub channel_name: String,
    pub owner: String,
    /// Pubkey → display name.
    pub names: HashMap<String, String>,
    /// Agents present, spoken or not.
    pub attendees: Vec<String>,
    pub utterances: Vec<Utterance>,
}

/// The written-up meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Minutes {
    pub file_name: String,
    /// Everything said, for the file on this Mac.
    pub full_text: String,
    /// The owner's requests for the channel; `None` when the owner said nothing.
    pub posted: Option<String>,
    /// The agent the posted minutes mention.
    pub lead: Option<String>,
}

fn name_of(meeting: &Meeting, pubkey: &str) -> String {
    meeting
        .names
        .get(&pubkey.to_ascii_lowercase())
        .cloned()
        .unwrap_or_else(|| format!("{}…", &pubkey[..pubkey.len().min(8)]))
}

/// `8:05 م` — the owner's clock, twelve-hour with an Arabic day half.
fn clock(at: i64, offset: FixedOffset) -> String {
    offset
        .timestamp_opt(at, 0)
        .single()
        .map(|time| {
            let half = if time.format("%P").to_string() == "am" {
                "ص"
            } else {
                "م"
            };
            format!("{} {half}", time.format("%-I:%M"))
        })
        .unwrap_or_default()
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(40).collect();
    if out.is_empty() {
        "meeting".to_string()
    } else {
        out
    }
}

/// Write the meeting up. `None` when nothing was said.
pub(crate) fn write_minutes(meeting: &Meeting, offset: FixedOffset) -> Option<Minutes> {
    let mut said: Vec<&Utterance> = meeting
        .utterances
        .iter()
        .filter(|u| !u.text.trim().is_empty())
        .collect();
    if said.is_empty() {
        return None;
    }
    said.sort_by_key(|u| u.at);
    let (start, end) = (said[0].at, said[said.len() - 1].at);
    let minutes = ((end - start) as f64 / 60.0).ceil().max(1.0) as i64;
    let day = offset
        .timestamp_opt(start, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default();

    let owner = meeting.owner.to_ascii_lowercase();
    let owner_name = name_of(meeting, &owner);
    let mut present: Vec<String> = vec![owner.clone()];
    for pubkey in meeting
        .attendees
        .iter()
        .chain(said.iter().map(|u| &u.speaker))
    {
        let pubkey = pubkey.to_ascii_lowercase();
        if !present.contains(&pubkey) {
            present.push(pubkey);
        }
    }
    let present_names = present
        .iter()
        .map(|p| name_of(meeting, p))
        .collect::<Vec<_>>()
        .join("، ");
    let lead = present
        .iter()
        .find(|p| {
            **p != owner
                && name_of(meeting, p)
                    .to_ascii_uppercase()
                    .starts_with(LEAD_AGENT)
        })
        .cloned();

    let channel = if meeting.channel_name.is_empty() {
        "اجتماع".to_string()
    } else {
        meeting.channel_name.clone()
    };
    let mut full = format!(
        "# محضر اجتماع — {channel}\n\n{day} · من {} إلى {} ({minutes} دقيقة)\n\nالحضور: {present_names}\n\n## المحادثة\n\n",
        clock(start, offset),
        clock(end, offset),
    );
    for u in &said {
        full.push_str(&format!(
            "- **{} {}:** {}\n",
            clock(u.at, offset),
            name_of(meeting, &u.speaker),
            u.text.trim()
        ));
    }

    let owner_lines: Vec<String> = said
        .iter()
        .filter(|u| u.speaker.eq_ignore_ascii_case(&owner))
        .map(|u| format!("• {} — {}", clock(u.at, offset), u.text.trim()))
        .collect();
    let posted = (!owner_lines.is_empty()).then(|| {
        let mut body = String::new();
        let mut cut = false;
        for line in &owner_lines {
            if body.chars().count() + line.chars().count() > MAX_POSTED_CHARS {
                cut = true;
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        let mut text = format!(
            "📝 محضر الاجتماع — {channel} — {day} {}\nالمدة {minutes} دقيقة · الحضور: {present_names}\n\nطلبات {owner_name} وقراراته:\n{body}",
            clock(start, offset)
        );
        if cut {
            text.push_str("… والبقية في المحضر الكامل المحفوظ على الجهاز.\n");
        }
        if let Some(lead) = &lead {
            text.push_str(&format!(
                "\n@{} هذا محضر الاجتماع. حوّل طلبات {owner_name} إلى مهام وخذها بعين الاعتبار في الخطة.",
                name_of(meeting, lead)
            ));
        }
        text
    });

    let stamp = offset
        .timestamp_opt(start, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d_%H%M").to_string())
        .unwrap_or_else(|| start.to_string());
    Some(Minutes {
        file_name: format!("{stamp}_{}.md", slug(&meeting.channel_name)),
        full_text: full,
        posted,
        lead,
    })
}

/// Read the meeting before its channel is archived, then write it up in the
/// background. Never fails the ending: every error is logged and dropped.
pub(crate) async fn record_meeting(
    parent_channel_id: &str,
    ephemeral_channel_id: &str,
    state: &AppState,
) {
    if parent_channel_id.is_empty() || ephemeral_channel_id.is_empty() {
        return;
    }
    let Ok(owner) = state.keys.lock().map(|keys| keys.public_key().to_hex()) else {
        return;
    };
    let messages = [serde_json::json!({
        "kinds": [9],
        "#h": [ephemeral_channel_id],
        "limit": MAX_MESSAGES,
    })];
    let read = tokio::join!(
        tokio::time::timeout(READ_TIMEOUT, query_relay(state, &messages)),
        tokio::time::timeout(
            READ_TIMEOUT,
            super::relay_api::fetch_channel_members(ephemeral_channel_id, Some("bot"), state),
        ),
    );
    let events = match read.0 {
        Ok(Ok(events)) => events,
        Ok(Err(error)) => {
            eprintln!("buzz-desktop: meeting minutes: could not read the meeting: {error}");
            return;
        }
        Err(_) => {
            eprintln!("buzz-desktop: meeting minutes: reading the meeting timed out");
            return;
        }
    };
    let attendees = match read.1 {
        Ok(Ok(members)) => members,
        _ => Vec::new(),
    };
    let utterances: Vec<Utterance> = events
        .iter()
        .map(|event| Utterance {
            at: event.created_at.as_secs() as i64,
            speaker: event.pubkey.to_hex(),
            text: event.content.clone(),
        })
        .collect();
    if utterances.iter().all(|u| u.text.trim().is_empty()) {
        return;
    }
    let Some(app) = state
        .app_handle
        .lock()
        .ok()
        .and_then(|handle| handle.clone())
    else {
        return;
    };
    let parent = parent_channel_id.to_string();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let meeting = Meeting {
            channel_name: String::new(),
            owner,
            names: HashMap::new(),
            attendees,
            utterances,
        };
        let meeting = with_names(meeting, &parent, &state, &app).await;
        let offset = Local::now().offset().fix();
        let Some(minutes) = write_minutes(&meeting, offset) else {
            return;
        };
        save_minutes(&app, &minutes);
        if let Some(text) = &minutes.posted {
            post_minutes(&parent, text, minutes.lead.as_deref(), &state).await;
        }
    });
}

/// Fill in who everyone is and what the channel is called.
async fn with_names(
    mut meeting: Meeting,
    parent_channel_id: &str,
    state: &AppState,
    app: &tauri::AppHandle,
) -> Meeting {
    if let Ok(records) = crate::managed_agents::load_managed_agents(app) {
        for record in records {
            let name = record
                .name
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                meeting
                    .names
                    .insert(record.pubkey.to_ascii_lowercase(), name);
            }
        }
    }
    let mut people: Vec<String> = vec![meeting.owner.clone()];
    for pubkey in meeting
        .attendees
        .iter()
        .chain(meeting.utterances.iter().map(|u| &u.speaker))
    {
        if !people.contains(pubkey) {
            people.push(pubkey.clone());
        }
    }
    let profiles =
        serde_json::json!({ "kinds": [0], "authors": people, "limit": people.len() * 2 });
    let channel = serde_json::json!({ "kinds": [39000], "#d": [parent_channel_id], "limit": 1 });
    let filters = [profiles, channel];
    let found = tokio::time::timeout(NAMES_TIMEOUT, query_relay(state, &filters))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    for event in found {
        if event.kind.as_u16() == 39000 {
            if let Some(name) = event.tags.iter().find_map(|tag| {
                let slice = tag.as_slice();
                (slice.first().map(String::as_str) == Some("name"))
                    .then(|| slice.get(1).cloned())
                    .flatten()
            }) {
                meeting.channel_name = name;
            }
            continue;
        }
        let pubkey = event.pubkey.to_hex();
        if meeting.names.contains_key(&pubkey) {
            continue;
        }
        if let Ok(profile) = serde_json::from_str::<serde_json::Value>(&event.content) {
            let name = ["display_name", "displayName", "name"]
                .iter()
                .find_map(|key| profile.get(*key).and_then(|v| v.as_str()))
                .map(str::trim)
                .filter(|name| !name.is_empty());
            if let Some(name) = name {
                meeting.names.insert(pubkey, name.to_string());
            }
        }
    }
    meeting
}

fn save_minutes(app: &tauri::AppHandle, minutes: &Minutes) {
    let Ok(dir) = app
        .path()
        .app_data_dir()
        .map(|dir| dir.join("meeting-notes"))
    else {
        return;
    };
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(&minutes.file_name), &minutes.full_text));
    if let Err(error) = written {
        eprintln!("buzz-desktop: meeting minutes not saved: {error}");
    }
}

async fn post_minutes(parent_channel_id: &str, text: &str, lead: Option<&str>, state: &AppState) {
    let Ok(channel) = super::relay_api::parse_channel_uuid(parent_channel_id) else {
        return;
    };
    let mentions: Vec<&str> = lead.into_iter().collect();
    let relay_base = crate::relay::relay_api_base_url_with_override(state);
    let builder = match events::build_message(
        channel,
        text,
        None,
        &mentions,
        &[],
        &[],
        &[],
        &[],
        None,
        &relay_base,
    ) {
        Ok(builder) => builder,
        Err(error) => {
            eprintln!("buzz-desktop: meeting minutes message: {error}");
            return;
        }
    };
    if let Err(error) = submit_event(builder, state).await {
        eprintln!("buzz-desktop: meeting minutes not posted: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "aa11";
    const CORTEX: &str = "bb22";
    const AXEL: &str = "cc33";

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn meeting(utterances: Vec<(i64, &str, &str)>) -> Meeting {
        Meeting {
            channel_name: "product-launch-management".into(),
            owner: OWNER.into(),
            names: HashMap::from([
                (OWNER.into(), "Sara".into()),
                (CORTEX.into(), "CORTEX".into()),
                (AXEL.into(), "Axel".into()),
            ]),
            attendees: vec![CORTEX.into(), AXEL.into()],
            utterances: utterances
                .into_iter()
                .map(|(at, speaker, text)| Utterance {
                    at,
                    speaker: speaker.into(),
                    text: text.into(),
                })
                .collect(),
        }
    }

    // 2026-09-24 17:05:00 UTC
    const T0: i64 = 1_790_269_500;

    #[test]
    fn nothing_said_writes_nothing() {
        assert_eq!(write_minutes(&meeting(vec![]), utc()), None);
        assert_eq!(
            write_minutes(&meeting(vec![(T0, OWNER, "  ")]), utc()),
            None
        );
    }

    #[test]
    fn the_file_keeps_everything_in_order_and_the_post_keeps_the_owners_requests() {
        let minutes = write_minutes(
            &meeting(vec![
                (T0 + 120, CORTEX, "Understood."),
                (T0, OWNER, "نقفل الليلة، والسرعة أولاً"),
                (T0 + 300, OWNER, "سجّلوا الخطوة العاشرة جزئية"),
                (T0 + 60, AXEL, "Step ten needs sources."),
            ]),
            utc(),
        )
        .expect("minutes");
        assert_eq!(
            minutes.file_name,
            "2026-09-24_1705_product-launch-management.md"
        );
        let order: Vec<usize> = ["نقفل الليلة", "Step ten", "Understood", "سجّلوا"]
            .iter()
            .map(|needle| minutes.full_text.find(needle).expect(needle))
            .collect();
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "{}",
            minutes.full_text
        );
        assert!(minutes.full_text.contains("من 5:05 م إلى 5:10 م (5 دقيقة)"));
        assert!(minutes.full_text.contains("الحضور: Sara، CORTEX، Axel"));

        let posted = minutes.posted.expect("posted");
        assert!(posted.contains("• 5:05 م — نقفل الليلة، والسرعة أولاً"));
        assert!(posted.contains("• 5:10 م — سجّلوا الخطوة العاشرة جزئية"));
        assert!(
            !posted.contains("Understood"),
            "only the owner's words are posted"
        );
        assert!(posted.ends_with(
            "@CORTEX هذا محضر الاجتماع. حوّل طلبات Sara إلى مهام وخذها بعين الاعتبار في الخطة."
        ));
        assert_eq!(minutes.lead.as_deref(), Some(CORTEX));
    }

    #[test]
    fn a_meeting_without_the_lead_is_posted_without_a_mention() {
        let mut m = meeting(vec![(T0, OWNER, "ابدأ")]);
        m.attendees = vec![AXEL.into()];
        let minutes = write_minutes(&m, utc()).expect("minutes");
        assert_eq!(minutes.lead, None);
        assert!(!minutes.posted.expect("posted").contains('@'));
    }

    #[test]
    fn when_only_agents_spoke_the_file_is_written_but_nothing_is_posted() {
        let minutes = write_minutes(&meeting(vec![(T0, AXEL, "status")]), utc()).expect("minutes");
        assert_eq!(minutes.posted, None);
        assert!(minutes.full_text.contains("status"));
    }

    #[test]
    fn a_long_meeting_is_cut_in_the_channel_but_kept_whole_in_the_file() {
        let words = "كلام ".repeat(400);
        let said: Vec<(i64, &str, &str)> = (0..10)
            .map(|i| (T0 + i * 30, OWNER, words.as_str()))
            .collect();
        let minutes = write_minutes(&meeting(said), utc()).expect("minutes");
        let posted = minutes.posted.expect("posted");
        assert!(posted.contains("والبقية في المحضر الكامل"));
        assert!(posted.chars().count() < MAX_POSTED_CHARS + 600);
        assert_eq!(minutes.full_text.matches("كلام").count(), 10 * 400);
    }

    #[test]
    fn unknown_speakers_get_a_short_key_and_odd_channel_names_a_safe_file_name() {
        let mut m = meeting(vec![(T0, "ffee99887766", "hello")]);
        m.channel_name = "نقاش سريع!".into();
        let minutes = write_minutes(&m, utc()).expect("minutes");
        assert!(minutes.full_text.contains("ffee9988…:"));
        assert_eq!(minutes.file_name, "2026-09-24_1705_meeting.md");
        assert_eq!(slug("Product Dev -- Mgmt"), "product-dev-mgmt");
    }

    #[test]
    fn the_clock_uses_the_owners_offset_and_arabic_halves() {
        let utc_plus_3 = FixedOffset::east_opt(3 * 3600).unwrap();
        assert_eq!(clock(T0, utc_plus_3), "8:05 م");
        assert_eq!(clock(T0 - 12 * 3600, utc_plus_3), "8:05 ص");
    }
}
