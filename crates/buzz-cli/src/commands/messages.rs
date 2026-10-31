use buzz_sdk::{DeleteMessageOptions, DiffMeta, ThreadRef, VoteDirection};
use nostr::PublicKey;
use uuid::Uuid;

use crate::client::{normalize_events, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{
    infer_language, parse_event_id, parse_uuid, read_or_stdin, truncate_diff,
    validate_content_size, validate_hex64, validate_uuid, MAX_DIFF_BYTES,
};
use buzz_sdk::mentions::{
    extract_at_mentions_with_known, extract_nostr_uris, strip_code_regions, MENTION_CAP,
};

/// Extract the thread root event ID from a Nostr tag array.
///
/// Delegates marker parsing and collapse to [`buzz_core::nip10`] (shared with
/// relay ingest and ACP) so id-validity, marker selection, and top-level
/// classification cannot drift:
/// - A `root`+`reply` parent returns its root event ID.
/// - A `reply`-only parent returns the reply target (a direct reply's parent IS
///   the root).
/// - A root-only or marker-less parent returns `None` (it is top-level and its
///   own root).
fn find_root_from_tags(tags: &serde_json::Value) -> Option<String> {
    let parts: Vec<Vec<String>> = tags
        .as_array()?
        .iter()
        .filter_map(|tag| {
            tag.as_array().map(|a| {
                a.iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            })
        })
        .collect();
    buzz_core::nip10::parse_thread_markers_from_parts(parts.iter().map(Vec::as_slice))
        .resolve()
        .map(|(root, _)| root)
}

fn thread_ref_from_parent_tags(
    parent_eid: nostr::EventId,
    parent_event_id: &str,
    tags: &serde_json::Value,
) -> Result<ThreadRef, CliError> {
    let root_eid = match find_root_from_tags(tags) {
        Some(root_hex) if root_hex != parent_event_id => parse_event_id(&root_hex)?,
        _ => parent_eid,
    };

    Ok(ThreadRef {
        root_event_id: root_eid,
        parent_event_id: parent_eid,
    })
}

/// Build a `ThreadRef` for a reply, given the immediate parent's event ID.
///
/// Fetches the parent event from the relay and inspects its NIP-10 `e` tags to
/// determine the thread root:
/// - Direct reply (parent is top-level): `root == parent`.
/// - Nested reply: `root` is the parent's own root marker; `parent` is unchanged.
///
/// Ensures CLI-sent replies thread correctly using the same NIP-10 logic.
async fn fetch_event(client: &BuzzClient, event_id: &str) -> Result<serde_json::Value, CliError> {
    let filter = serde_json::json!({ "ids": [event_id], "limit": 1 });
    let raw = client.query(&filter).await?;
    let events: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse query response: {e}")))?;
    events
        .as_array()
        .and_then(|events| events.first())
        .cloned()
        .ok_or_else(|| CliError::NotFound(format!("event {event_id} not found")))
}

async fn resolve_thread_ref(
    client: &BuzzClient,
    parent_event_id: &str,
) -> Result<ThreadRef, CliError> {
    let event = fetch_event(client, parent_event_id).await?;
    thread_ref_from_event(parent_event_id, &event)
}

fn thread_ref_from_event(event_id: &str, event: &serde_json::Value) -> Result<ThreadRef, CliError> {
    let parent_eid = parse_event_id(event_id)?;
    let tags = event
        .get("tags")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    thread_ref_from_parent_tags(parent_eid, event_id, &tags)
}

/// Resolve the channel UUID for an event by querying for it via POST /query.
/// Extracts the `h` tag value from the returned event's tags.
fn channel_id_from_event(event_id: &str, event: &serde_json::Value) -> Result<Uuid, CliError> {
    let tags = event
        .get("tags")
        .and_then(|tags| tags.as_array())
        .ok_or_else(|| CliError::Other("event missing 'tags' field".into()))?;
    tags.iter()
        .filter_map(|tag| tag.as_array())
        .find(|tag| tag.first().and_then(|value| value.as_str()) == Some("h"))
        .and_then(|tag| tag.get(1))
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            CliError::Other(format!(
                "event {event_id} has no h-tag — cannot determine channel"
            ))
        })
        .and_then(|channel_id| {
            Uuid::parse_str(channel_id).map_err(|_| {
                CliError::Other(format!("event h-tag is not a valid UUID: {channel_id}"))
            })
        })
}

async fn resolve_channel_id(client: &BuzzClient, event_id: &str) -> Result<Uuid, CliError> {
    let event = fetch_event(client, event_id).await?;
    channel_id_from_event(event_id, &event)
}

fn resolve_names_to_pubkeys(
    names: &[String],
    name_to_pubkeys: &std::collections::HashMap<String, Vec<String>>,
    has_explicit_mentions: bool,
) -> Result<Vec<String>, CliError> {
    let mut resolved = Vec::new();
    for name in names {
        match name_to_pubkeys
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            [pubkey] => resolved.push(pubkey.clone()),
            [] if has_explicit_mentions => {}
            [] => {
                return Err(CliError::Usage(format!(
                    "mention '@{name}' does not match a current channel member; retry with --mention <pubkey>"
                )))
            }
            _ if has_explicit_mentions => {}
            candidates => {
                return Err(CliError::Usage(format!(
                    "mention '@{name}' is ambiguous; candidates: {}. Retry with --mention <pubkey>",
                    candidates.join(", ")
                )))
            }
        }
    }
    Ok(resolved)
}

/// Resolve mention text against the channel membership snapshot.
///
/// Returns both the current member set and uniquely name-resolved pubkeys.
/// Lookup failures are fatal when mention processing is requested: publishing
/// visible mention text without its intended `p` tag is worse than not sending.
async fn resolve_content_mentions(
    client: &BuzzClient,
    channel_id: &str,
    content: &str,
    has_explicit_mentions: bool,
) -> Result<(Vec<String>, Vec<String>), CliError> {
    let stripped = strip_code_regions(content);
    let addressee = opening_addressee(&stripped);
    if !stripped.contains('@') && !has_explicit_mentions && addressee.is_none() {
        return Ok((vec![], vec![]));
    }

    let members_filter = serde_json::json!({
        "kinds": [39002],
        "#d": [channel_id],
        "limit": 1,
    });
    let member_pubkeys = fetch_member_pubkeys(client, &members_filter)
        .await
        .ok_or_else(|| {
            CliError::Other("could not load channel membership for mention preflight".into())
        })?;

    if !stripped.contains('@') && addressee.is_none() {
        return Ok((member_pubkeys, vec![]));
    }

    let profiles_filter = serde_json::json!({
        "kinds": [0],
        "authors": member_pubkeys,
        "limit": member_pubkeys.len(),
    });
    let profile_events = fetch_events(client, &profiles_filter)
        .await
        .ok_or_else(|| {
            CliError::Other("could not load member profiles for mention resolution".into())
        })?;

    let mut name_to_pubkeys: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut display_names = Vec::new();
    let mut named_members: Vec<(String, String)> = Vec::new();
    for e in &profile_events {
        let Some(pubkey) = e.get("pubkey").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(content_json) = e.get("content").and_then(|v| v.as_str()) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(content_json) else {
            continue;
        };
        let Some(name) = v
            .get("display_name")
            .or_else(|| v.get("name"))
            .and_then(|n| n.as_str())
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        name_to_pubkeys
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(pubkey.to_string());
        display_names.push(name.to_string());
        named_members.push((pubkey.to_string(), name.to_string()));
    }

    let known_refs: Vec<&str> = display_names.iter().map(String::as_str).collect();
    let names = extract_at_mentions_with_known(&stripped, &known_refs);
    let mut resolved = if stripped.contains('@') {
        resolve_names_to_pubkeys(&names, &name_to_pubkeys, has_explicit_mentions)?
    } else {
        Vec::new()
    };
    // A message that opens by addressing a teammate reaches them as a mention
    // would. Without this, "CORTEX، …" from a manager woke nobody: only a
    // tagged mention wakes an agent, so the one being asked never saw it.
    if let Some(addressee) = addressee.as_deref() {
        let own = client.keys().public_key().to_hex();
        if let Some(pubkey) = resolve_addressee(addressee, &named_members, &own) {
            if !resolved.contains(&pubkey) {
                resolved.push(pubkey);
            }
        }
    }
    Ok((member_pubkeys, resolved))
}

/// Arabic spellings of teammates' first names — the table the owner's own
/// composer uses for the same purpose (`nameCalls.ts` on the desktop).
const ARABIC_FIRST_NAMES: &[(&str, &[&str])] = &[
    ("aegis", &["ايجس", "ايجيس"]),
    ("atlas", &["اطلس"]),
    ("axel", &["اكسل", "اكسيل"]),
    ("byte", &["بايت"]),
    ("comply", &["كومبلاي", "كمبلاي"]),
    ("cortex", &["كورتكس", "كورتيكس"]),
    ("forge", &["فورج", "فورق"]),
    ("judge", &["جادج", "جدج"]),
    ("kernel", &["كيرنل", "كرنل"]),
    ("nexus", &["نيكسس", "نكسس", "نيكسوس"]),
    ("nova", &["نوفا"]),
    ("operator", &["اوبريتر", "اوبريتور", "اوبرايتر"]),
    ("orion", &["اوريون", "اوريان"]),
    ("pilot", &["بايلوت", "بايلت"]),
    ("pixel", &["بكسل", "بيكسل"]),
    ("prism", &["بريزم", "بريسم"]),
    ("probe", &["بروب"]),
    ("quill", &["كويل"]),
    ("relay", &["ريلاي", "ريلي"]),
    ("sage", &["سيج", "ساقا", "سايج", "ساغا"]),
    ("scope", &["سكوب"]),
    ("scribe", &["سكرايب", "سكريب"]),
    (
        "signal",
        &[
            "سيجنال",
            "سقنل",
            "سيقنل",
            "سغنل",
            "سجنال",
            "سيغنال",
            "سقنال",
        ],
    ),
    ("syntax", &["سينتاكس", "سنتاكس"]),
    ("terra", &["تيرا"]),
    ("value", &["فاليو"]),
    ("vault", &["فولت"]),
    ("vector", &["فكتور", "فيكتور"]),
];

/// The teammate a message opens by addressing: "CORTEX، …", "Pilot: …",
/// "يا كورتكس، …" — the first line's opening words before a comma, an Arabic
/// comma or a colon.
///
/// Only the opening counts. That is where a person is addressed, and it keeps
/// a name said in passing — "as Pilot noted" — from waking an agent that was
/// only being talked about. Three words at most, so a sentence that merely
/// contains a comma is not read as a name.
fn opening_addressee(content: &str) -> Option<String> {
    let line = content.trim_start().lines().next()?.trim();
    let line = line.strip_prefix("يا ").unwrap_or(line).trim_start();
    let end = line.find([',', '،', ':'])?;
    let candidate = line[..end].trim();
    let words = candidate.split_whitespace().count();
    if candidate.is_empty() || words > 3 || candidate.chars().count() > 40 {
        return None;
    }
    Some(candidate.to_string())
}

/// Case and spelling variants that name the same person compare equal:
/// ASCII case, the alef and hamza forms, taa marbuta, alef maqsura, tatweel
/// and short vowel marks.
fn normalize_name(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(*c, '\u{0640}' | '\u{064B}'..='\u{065F}'))
        .map(|c| match c {
            'أ' | 'إ' | 'آ' => 'ا',
            'ة' => 'ه',
            'ى' => 'ي',
            other => other.to_ascii_lowercase(),
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// The one member an opening addressee names, by full display name, first
/// name, or an Arabic spelling of the first name — never the sender, and
/// never a guess between two.
fn resolve_addressee(
    addressee: &str,
    members: &[(String, String)],
    own_pubkey: &str,
) -> Option<String> {
    let wanted = normalize_name(addressee);
    let matches: Vec<&String> = members
        .iter()
        .filter(|(pubkey, _)| pubkey != own_pubkey)
        .filter(|(_, name)| {
            let first = normalize_name(name.split_whitespace().next().unwrap_or(""));
            let arabic = ARABIC_FIRST_NAMES
                .iter()
                .find(|(english, _)| *english == first)
                .map_or(&[][..], |(_, spellings)| *spellings);
            wanted == normalize_name(name)
                || wanted == first
                || arabic
                    .iter()
                    .any(|spelling| normalize_name(spelling) == wanted)
        })
        .map(|(pubkey, _)| pubkey)
        .collect();
    match matches.as_slice() {
        [only] => Some((*only).clone()),
        _ => None,
    }
}

fn normalize_explicit_mentions(values: &[String]) -> Result<Vec<String>, CliError> {
    let mut normalized = Vec::new();
    for value in values {
        let pubkey = PublicKey::parse(value.trim())
            .map_err(|_| CliError::Usage(format!("invalid --mention pubkey: {value}")))?;
        let hex = pubkey.to_hex();
        if !normalized.contains(&hex) {
            normalized.push(hex);
        }
    }
    if normalized.len() > MENTION_CAP {
        return Err(CliError::Usage(format!(
            "too many --mention values (max {MENTION_CAP})"
        )));
    }
    Ok(normalized)
}

fn merge_message_mentions(
    explicit: &[String],
    uri_pubkeys: &[String],
    auto_resolved: &[String],
) -> Result<Vec<String>, CliError> {
    let mut mentions = Vec::new();
    for pubkey in explicit
        .iter()
        .chain(uri_pubkeys.iter())
        .chain(auto_resolved.iter())
    {
        if !mentions.contains(pubkey) {
            mentions.push(pubkey.clone());
        }
    }
    if mentions.len() > MENTION_CAP {
        return Err(CliError::Usage(format!(
            "too many unique message mentions (max {MENTION_CAP})"
        )));
    }
    Ok(mentions)
}

fn missing_members(mentions: &[String], members: &[String]) -> Vec<String> {
    let members: std::collections::HashSet<&str> = members.iter().map(String::as_str).collect();
    mentions
        .iter()
        .filter(|pk| !members.contains(pk.as_str()))
        .cloned()
        .collect()
}

fn event_mention_pubkeys(event: &nostr::Event) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("p"))
                .then(|| parts.get(1).cloned())
                .flatten()
        })
        .collect()
}

/// Fetch raw events for `filter` via the relay's `/query` endpoint.
/// Returns `None` on any I/O or parse failure.
async fn fetch_events(
    client: &BuzzClient,
    filter: &serde_json::Value,
) -> Option<Vec<serde_json::Value>> {
    let raw = client.query(filter).await.ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    parsed.as_array().cloned()
}

/// Extract member pubkeys (the `p` tag values) from a single 39002 event.
async fn fetch_member_pubkeys(
    client: &BuzzClient,
    filter: &serde_json::Value,
) -> Option<Vec<String>> {
    let events = fetch_events(client, filter).await?;
    Some(parse_member_pubkeys(events.first()?))
}

/// Parse member pubkeys from a kind 39002 event JSON value.
///
/// Filters and canonicalizes via `nostr::PublicKey::from_hex` — matching
/// MCP's typed-Nostr behavior so both surfaces accept exactly the same
/// pubkeys. Pure helper, split out for testing.
fn parse_member_pubkeys(event: &serde_json::Value) -> Vec<String> {
    let Some(tags) = event.get("tags").and_then(|t| t.as_array()) else {
        return vec![];
    };
    tags.iter()
        .filter_map(|t| {
            let arr = t.as_array()?;
            if arr.first()?.as_str()? != "p" {
                return None;
            }
            let pk = arr.get(1)?.as_str()?;
            PublicKey::from_hex(pk).ok().map(|k| k.to_hex())
        })
        .collect()
}

fn format_events(normalized: &str, format: &crate::OutputFormat) -> String {
    match format {
        crate::OutputFormat::Compact => {
            let events: Vec<serde_json::Value> =
                serde_json::from_str(normalized).unwrap_or_default();
            let compact: Vec<serde_json::Value> = events
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.get("id").cloned().unwrap_or_default(),
                        "content": e.get("content").cloned().unwrap_or_default(),
                        "created_at": e.get("created_at").cloned().unwrap_or_default(),
                    })
                })
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
        crate::OutputFormat::Json => normalized.to_string(),
    }
}

pub async fn cmd_get_messages(
    client: &BuzzClient,
    channel_id: &str,
    limit: Option<u32>,
    before: Option<i64>,
    since: Option<i64>,
    kinds: Option<&str>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let limit = limit.unwrap_or(50).min(200);

    let mut filter = serde_json::json!({
        "kinds": [9, 40002, 40008, 45001, 45003],
        "#h": [channel_id],
        "limit": limit
    });

    // If specific kinds requested, override
    if let Some(k) = kinds {
        let kind_list: Vec<u64> = k.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if !kind_list.is_empty() {
            filter["kinds"] = serde_json::json!(kind_list);
        }
    }

    if let Some(b) = before {
        filter["until"] = serde_json::json!(b);
    }
    if let Some(s) = since {
        filter["since"] = serde_json::json!(s);
    }

    let resp = client.query(&filter).await?;
    let mut events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    events.sort_by_key(|e| e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0));
    let normalized = normalize_events(&events);
    println!("{}", format_events(&normalized, format));
    Ok(())
}

pub fn resolve_thread_target(
    expected_channel_id: Uuid,
    event_id: &str,
    expected_root_id: Option<&str>,
    selected_event: &serde_json::Value,
) -> Result<String, CliError> {
    let actual_channel_id = channel_id_from_event(event_id, selected_event)?;
    if actual_channel_id != expected_channel_id {
        return Err(CliError::Usage(format!(
            "event {event_id} does not belong to channel {expected_channel_id}"
        )));
    }
    let root_event_id = thread_ref_from_event(event_id, selected_event)?
        .root_event_id
        .to_hex();
    if expected_root_id.is_some_and(|expected| expected != root_event_id) {
        return Err(CliError::Usage(
            "Buzz message link thread root does not match the selected message".into(),
        ));
    }
    Ok(root_event_id)
}

pub async fn cmd_get_thread(
    client: &BuzzClient,
    channel_id: &str,
    event_id: &str,
    expected_root_id: Option<&str>,
    limit: Option<u32>,
    depth_limit: Option<u32>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    let expected_channel_id = parse_uuid(channel_id)?;
    validate_hex64(event_id)?;
    let selected_event = fetch_event(client, event_id).await?;
    let root_event_id = resolve_thread_target(
        expected_channel_id,
        event_id,
        expected_root_id,
        &selected_event,
    )?;
    let limit = limit.unwrap_or(100).min(500);

    let mut reply_filter = serde_json::json!({
        "kinds": [9, 40002, 40003, 40008, 45003],
        "#h": [channel_id],
        "#e": [root_event_id.as_str()],
        "limit": limit
    });
    if let Some(d) = depth_limit {
        reply_filter["depth_limit"] = serde_json::json!(d);
    }
    let root_filter = serde_json::json!({
        "ids": [root_event_id.as_str()],
        "#h": [channel_id],
        "limit": 1
    });
    let resp = client.query_multi(&[reply_filter, root_filter]).await?;
    let mut events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    events.sort_by_key(|event| {
        event
            .get("created_at")
            .and_then(|value| value.as_u64())
            .unwrap_or(0)
    });
    let normalized = normalize_events(&events);
    println!("{}", format_events(&normalized, format));
    Ok(())
}

pub async fn cmd_search(
    client: &BuzzClient,
    query: Option<&str>,
    author: Option<&str>,
    since: Option<i64>,
    limit: Option<u32>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    if query.is_none() && author.is_none() {
        return Err(CliError::Usage(
            "at least one of --query or --author is required".into(),
        ));
    }
    let limit = limit.unwrap_or(20).min(100);

    let author_hex = match author {
        Some(a) => Some(resolve_author(client, a).await?),
        None => None,
    };

    let mut filter = serde_json::json!({
        "kinds": [9, 40002, 45001, 45003],
        "limit": limit
    });
    if let Some(q) = query {
        filter["search"] = serde_json::json!(q);
    }
    if let Some(ref pk) = author_hex {
        filter["authors"] = serde_json::json!([pk]);
    }
    if let Some(s) = since {
        filter["since"] = serde_json::json!(s);
    }
    let resp = client.query(&filter).await?;
    let mut events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    // The full-text path returns relevance order; a pure author/time query has
    // no relevance, so present newest-first like `messages get`.
    if query.is_none() {
        events.sort_by_key(|e| {
            std::cmp::Reverse(e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0))
        });
    }
    let normalized = normalize_events(&events);
    println!("{}", format_events(&normalized, format));
    Ok(())
}

/// Resolve an `--author` value to a 64-char hex pubkey.
///
/// Accepts, in order of precedence: 64-char hex (validated), an `npub1…`
/// bech32 key, or a display name resolved via NIP-50 profile search. A name
/// must match exactly one user (case-insensitive, on `display_name` or
/// `name`) — ambiguity is an error listing the candidates rather than a
/// silent mix of authors.
async fn resolve_author(client: &BuzzClient, author: &str) -> Result<String, CliError> {
    let author = author.trim();
    if author.len() == 64 && author.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(author.to_ascii_lowercase());
    }
    if author.starts_with("npub1") {
        return nostr::PublicKey::parse(author)
            .map(|pk| pk.to_hex())
            .map_err(|_| CliError::Usage(format!("invalid npub: {author}")));
    }

    // Display name → NIP-50 search on kind:0, exact case-insensitive match.
    let filter = serde_json::json!({
        "kinds": [0],
        "search": author,
        "limit": 100
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
    let mut matches = match_profiles_by_name(&events, author);
    match matches.len() {
        0 => Err(CliError::Usage(format!(
            "no user found with name '{author}' — pass a hex pubkey or npub instead"
        ))),
        1 => Ok(matches.remove(0).0),
        _ => {
            // Cap the candidate listing — some names are shared by dozens of
            // users, and an unbounded list turns the error into a wall of text.
            let shown = 5.min(matches.len());
            let mut listing: Vec<String> = matches[..shown]
                .iter()
                .map(|(pk, name)| format!("{name} ({pk})"))
                .collect();
            if matches.len() > shown {
                listing.push(format!("… and {} more", matches.len() - shown));
            }
            Err(CliError::Usage(format!(
                "name '{author}' is ambiguous — matches: {}. Pass a pubkey instead",
                listing.join(", ")
            )))
        }
    }
}

/// Exact case-insensitive profile match on `display_name` or `name` across
/// kind:0 events. Returns deduped `(pubkey, shown name)` pairs. Pure so the
/// name-resolution semantics are unit-testable without a relay.
fn match_profiles_by_name(events: &[serde_json::Value], name: &str) -> Vec<(String, String)> {
    let lower = name.to_ascii_lowercase();
    let mut matches: Vec<(String, String)> = Vec::new();
    for e in events {
        let Some(pubkey) = e.get("pubkey").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(content) = e
            .get("content")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        else {
            continue;
        };
        let display_name = content
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let plain_name = content.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if display_name.to_ascii_lowercase() == lower || plain_name.to_ascii_lowercase() == lower {
            let shown = if display_name.is_empty() {
                plain_name
            } else {
                display_name
            };
            matches.push((pubkey.to_string(), shown.to_string()));
        }
    }
    matches.sort();
    matches.dedup();
    matches
}

pub struct SendMessageParams {
    pub channel_id: String,
    pub content: String,
    pub kind: Option<u16>,
    pub reply_to: Option<String>,
    pub broadcast: bool,
    pub files: Vec<String>,
    pub mentions: Vec<String>,
    /// People copied in: delivered and recorded, never asked to act.
    pub cc: Vec<String>,
}

/// Record that this agent actually said something.
///
/// A turn that ends without a word is indistinguishable, from the outside,
/// from a turn that never ran: the person who asked sees silence either way.
/// The harness can tell the two apart only if something marks the moment a
/// message really went out, and the only process that knows that is this one.
///
/// Best effort by design. A receipt that cannot be written costs a notice the
/// owner might not have needed; failing the send over it would cost the
/// message itself, which is the thing that mattered.
fn record_send_receipt() {
    let Some(path) = std::env::var_os("BUZZ_SEND_RECEIPT") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // The content is irrelevant — the harness reads the modification time.
    // Writing rather than appending keeps the file from growing without end.
    let _ = std::fs::write(&path, b"sent\n");
}

/// Who this agent's own house is, from the environment the harness set.
///
/// `BUZZ_ACP_MANAGER` is the agent this one reports to; `BUZZ_ACP_SAME_HOUSE`
/// is everyone a message may reach without informing that manager — the
/// manager itself, peers under it, and this agent's own reports.
fn reporting_line_from_env() -> (Option<String>, std::collections::HashSet<String>) {
    let manager = std::env::var("BUZZ_ACP_MANAGER")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let house = std::env::var("BUZZ_ACP_SAME_HOUSE")
        .unwrap_or_default()
        .split(',')
        .map(|entry| entry.trim().to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .collect();
    (manager, house)
}

/// Copy this agent's manager when a message leaves its own house.
///
/// This is the difference between a policy and a suggestion. Written only into
/// prompts, "copy your manager on a cross-department request" is a rule every
/// model has to remember on every turn — and one that forgets sends a second
/// `@mention` to the manager instead, waking the person it meant merely to
/// inform. Applied here, at the single point every message passes through, the
/// reporting line holds regardless of which model is driving.
///
/// Nothing is added when the agent has no manager, when every mention is
/// inside its own house, or when the manager is already named. The owner is
/// never a crossing: talking to the person who owns the whole chart informs
/// nobody further up, because there is nobody further up.
fn manager_copy_for_crossing(
    mentions: &[String],
    already_cc: &[String],
    owner_hex: Option<&str>,
) -> Option<String> {
    let (manager, house) = reporting_line_from_env();
    let manager = manager?;
    let owner = owner_hex.map(|hex| hex.trim().to_ascii_lowercase());
    let crosses_out = mentions.iter().any(|pubkey| {
        let pubkey = pubkey.to_ascii_lowercase();
        pubkey != manager && Some(&pubkey) != owner.as_ref() && !house.contains(&pubkey)
    });
    if !crosses_out {
        return None;
    }
    if already_cc
        .iter()
        .any(|pubkey| pubkey.to_ascii_lowercase() == manager)
    {
        return None;
    }
    Some(manager)
}

pub async fn cmd_send_message(
    client: &BuzzClient,
    mut p: SendMessageParams,
) -> Result<(), CliError> {
    // Allow '-' to read content from stdin. This keeps callers from having to
    // jam shell-metacharacter-heavy text (backticks, $vars, etc.) through argv
    // quoting — the source of countless self-inflicted command-substitution
    // bugs for agent and human users alike.
    p.content = read_or_stdin(&p.content)?;
    validate_content_size(&p.content)?;
    if let Some(ref r) = p.reply_to {
        validate_hex64(r)?;
    }
    let channel_uuid = parse_uuid(&p.channel_id)?;

    let explicit_mentions = normalize_explicit_mentions(&p.mentions)?;
    let stripped = strip_code_regions(&p.content);
    let uri_pubkeys = extract_nostr_uris(&stripped);
    // Supplying any identity explicitly authorizes unresolved or ambiguous @Name text
    // as presentation-only, matching Desktop's separate visible-label and p-tag model.
    // Uniquely resolvable member names still add their own p-tags; callers must supply
    // every intended identity whose visible label cannot be resolved uniquely.
    let has_explicit_mentions = !explicit_mentions.is_empty() || !uri_pubkeys.is_empty();
    let (member_pubkeys, auto_resolved) =
        resolve_content_mentions(client, &p.channel_id, &p.content, has_explicit_mentions).await?;
    let mention_pubkeys = merge_message_mentions(&explicit_mentions, &uri_pubkeys, &auto_resolved)?;

    let missing = missing_members(&mention_pubkeys, &member_pubkeys);
    if !missing.is_empty() {
        return Err(CliError::Usage(
            serde_json::json!({
                "message": "mentioned pubkeys are not channel members; add them explicitly before retrying",
                "missing_member_pubkeys": missing,
                "add_member_command": format!("buzz channels add-member --channel {} --pubkey <pubkey> --role <member|bot>", p.channel_id),
            })
            .to_string(),
        ));
    }

    // Upload files and build imeta tags
    let mut media_tags: Vec<Vec<String>> = Vec::new();
    let mut media_content = String::new();
    for file_path in &p.files {
        let desc = client
            .upload_file(file_path)
            .await
            .map_err(|e| CliError::Other(format!("upload failed for {file_path}: {e}")))?;
        media_tags.push(crate::client::build_imeta_tag(&desc));
        if desc.mime_type.starts_with("video/") {
            media_content.push_str("\n![video](");
        } else {
            media_content.push_str("\n![image](");
        }
        media_content.push_str(&desc.url);
        media_content.push(')');
    }
    let final_content = if media_content.is_empty() {
        p.content.clone()
    } else {
        format!("{}{media_content}", p.content)
    };

    // Build thread ref if replying. `--reply-to` is the immediate parent; the
    // thread root is derived from the parent's NIP-10 tags via the relay.
    let thread_ref = if let Some(ref r) = p.reply_to {
        Some(resolve_thread_ref(client, r).await?)
    } else {
        None
    };

    let mention_refs: Vec<&str> = mention_pubkeys.iter().map(String::as_str).collect();
    // A CC is validated like a mention — a copy addressed to a malformed key
    // is a copy nobody receives — but it is never auto-resolved from the text.
    // Copying someone in is a deliberate act, not something prose should do
    // on the sender's behalf.
    let mut cc_pubkeys = normalize_explicit_mentions(&p.cc)?;
    if let Some(manager) = manager_copy_for_crossing(
        &mention_pubkeys,
        &cc_pubkeys,
        std::env::var("BUZZ_ACP_AGENT_OWNER").ok().as_deref(),
    ) {
        // Said out loud, because a copy the sender did not know it sent is a
        // copy it cannot account for when the manager asks about it.
        eprintln!(
            "note: copied your manager ({}) — this message leaves your department",
            &manager[..manager.len().min(8)]
        );
        cc_pubkeys.push(manager);
    }
    let cc_refs: Vec<&str> = cc_pubkeys.iter().map(String::as_str).collect();

    let builder = match p.kind {
        Some(45001) => {
            buzz_sdk::build_forum_post(channel_uuid, &final_content, &mention_refs, &media_tags)
                .map_err(|e| CliError::Other(format!("build_forum_post failed: {e}")))?
        }
        Some(45003) => {
            let tr = thread_ref.as_ref().ok_or_else(|| {
                CliError::Usage("--reply-to is required for forum comments (kind 45003)".into())
            })?;
            buzz_sdk::build_forum_comment(
                channel_uuid,
                &final_content,
                tr,
                &mention_refs,
                &media_tags,
            )
            .map_err(|e| CliError::Other(format!("build_forum_comment failed: {e}")))?
        }
        None | Some(9) => {
            // Scan final_content for `:shortcode:` patterns and attach NIP-30
            // emoji tags for any that resolve in the workspace palette.
            // Palette resolution is scoped to kind 9: forum builders (45001,
            // 45003) do not accept emoji_tags, so resolving early would pay
            // the relay query and immediately discard the result.
            // The fetch is skipped entirely when content has no `:`, keeping
            // plain sends at zero extra RTTs.  Palette resolution is
            // decorative enrichment — a fetch or parse failure must not block
            // delivery of a valid message; on error, degrade to no emoji tags
            // and log a diagnostic to stderr.
            let emoji_tags = if final_content.contains(':') {
                match crate::commands::emoji::resolve_emoji_tags_for_content(client, &final_content)
                    .await
                {
                    Ok(tags) => tags,
                    Err(e) => {
                        eprintln!(
                            "warning: emoji palette fetch failed ({e}); sending without emoji tags"
                        );
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };
            buzz_sdk::build_message_with_cc(
                channel_uuid,
                &final_content,
                thread_ref.as_ref(),
                &mention_refs,
                &cc_refs,
                p.broadcast,
                &media_tags,
                &emoji_tags,
            )
            .map_err(|e| CliError::Other(format!("build_message failed: {e}")))?
        }
        Some(k) => {
            return Err(CliError::Usage(format!(
                "--kind {k} is not supported (use 9, 45001, or 45003)"
            )))
        }
    };

    let event = client.sign_event(builder)?;
    let emitted_mentions = event_mention_pubkeys(&event);
    let resp = client.submit_event(event).await?;
    let mut output: serde_json::Value = serde_json::from_str(&normalize_write_response(&resp))
        .unwrap_or_else(|_| serde_json::json!({ "response": resp }));
    if let Some(object) = output.as_object_mut() {
        object.insert(
            "mention_pubkeys".into(),
            serde_json::json!(emitted_mentions),
        );
    }
    println!("{output}");
    record_send_receipt();
    Ok(())
}

pub struct SendDiffParams {
    pub channel_id: String,
    pub diff: String,
    pub repo_url: String,
    pub commit_sha: String,
    pub file_path: Option<String>,
    pub parent_commit_sha: Option<String>,
    pub source_branch: Option<String>,
    pub target_branch: Option<String>,
    pub pr_number: Option<u32>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub reply_to: Option<String>,
}

pub async fn cmd_send_diff_message(client: &BuzzClient, p: SendDiffParams) -> Result<(), CliError> {
    if let Some(r) = &p.reply_to {
        validate_hex64(r)?;
    }

    // Branch pairing: both or neither
    match (&p.source_branch, &p.target_branch) {
        (Some(_), None) | (None, Some(_)) => {
            return Err(CliError::Usage(
                "--source-branch and --target-branch must both be provided or both omitted".into(),
            ));
        }
        _ => {}
    }

    let channel_uuid = parse_uuid(&p.channel_id)?;

    // Read diff from stdin if "--diff -"
    let diff_content = read_or_stdin(&p.diff)?;

    // Truncate at 60 KiB hunk boundary
    let (diff, truncated) = truncate_diff(&diff_content, MAX_DIFF_BYTES);

    // Language inference: explicit flag wins, then infer from file path
    let language = p
        .language
        .clone()
        .or_else(|| p.file_path.as_deref().and_then(infer_language));

    // NIP-31 alt tag
    let alt = match (&p.file_path, &p.description) {
        (Some(fp), Some(desc)) => format!("Diff: {} — {}", fp, desc),
        (Some(fp), None) => format!("Diff: {}", fp),
        _ => "Diff".to_string(),
    };

    // `--reply-to` is the immediate parent; the thread root is derived from
    // the parent's NIP-10 tags via the relay.
    let thread_ref = if let Some(r) = &p.reply_to {
        Some(resolve_thread_ref(client, r).await?)
    } else {
        None
    };

    let branch = match (&p.source_branch, &p.target_branch) {
        (Some(src), Some(tgt)) => Some((src.clone(), tgt.clone())),
        _ => None,
    };

    let diff_meta = DiffMeta {
        repo_url: p.repo_url.clone(),
        commit_sha: p.commit_sha.clone(),
        file_path: p.file_path.clone(),
        parent_commit: p.parent_commit_sha.clone(),
        branch,
        pr_number: p.pr_number,
        language,
        description: p.description.clone(),
        truncated,
        alt_text: Some(alt),
    };

    let builder =
        buzz_sdk::build_diff_message(channel_uuid, &diff, &diff_meta, thread_ref.as_ref())
            .map_err(|e| CliError::Other(format!("build_diff_message failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    // A diff posted to the channel is an answer too. Without the receipt the
    // harness would take the turn for a silent one and send the agent's
    // closing text after it.
    record_send_receipt();
    Ok(())
}

pub async fn cmd_delete_message(
    client: &BuzzClient,
    event_id: &str,
    action_id: Option<Uuid>,
    reason_code: Option<&str>,
    public_reason: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;

    // Resolve channel_id from the event's h-tag
    let channel_uuid = resolve_channel_id(client, event_id).await?;
    let target_eid = parse_event_id(event_id)?;

    let builder = buzz_sdk::build_delete_message_with_options(
        channel_uuid,
        target_eid,
        DeleteMessageOptions {
            action_id,
            reason_code,
            public_reason,
        },
    )
    .map_err(|e| CliError::Other(format!("build_delete_message failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Edit a message you previously sent.
pub async fn cmd_edit_message(
    client: &BuzzClient,
    event_id: &str,
    content: &str,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    validate_content_size(content)?;

    // Resolve channel_id from the event's h-tag
    let channel_uuid = resolve_channel_id(client, event_id).await?;
    let target_eid = parse_event_id(event_id)?;

    let builder = buzz_sdk::build_edit(channel_uuid, target_eid, content)
        .map_err(|e| CliError::Other(format!("build_edit failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Vote on a forum post or comment.
pub async fn cmd_vote_on_post(
    client: &BuzzClient,
    event_id: &str,
    direction: &str,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    let vote_dir = match direction {
        "up" => VoteDirection::Up,
        "down" => VoteDirection::Down,
        _ => {
            return Err(CliError::Usage(format!(
                "--direction must be 'up' or 'down' (got: {direction})"
            )))
        }
    };

    // Resolve channel_id from the event's h-tag
    let channel_uuid = resolve_channel_id(client, event_id).await?;
    let target_eid = parse_event_id(event_id)?;

    let builder = buzz_sdk::build_vote(channel_uuid, target_eid, vote_dir)
        .map_err(|e| CliError::Other(format!("build_vote failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn dispatch(
    cmd: crate::MessagesCmd,
    client: &BuzzClient,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    use crate::MessagesCmd;
    match cmd {
        MessagesCmd::Send {
            channel,
            content,
            kind,
            reply_to,
            broadcast,
            files,
            mentions,
            cc,
        } => {
            cmd_send_message(
                client,
                SendMessageParams {
                    channel_id: channel,
                    content,
                    kind,
                    reply_to,
                    broadcast,
                    files,
                    mentions,
                    cc,
                },
            )
            .await
        }
        MessagesCmd::SendDiff {
            channel,
            diff,
            repo,
            commit,
            file,
            parent_commit,
            source_branch,
            target_branch,
            pr,
            lang,
            description,
            reply_to,
        } => {
            cmd_send_diff_message(
                client,
                SendDiffParams {
                    channel_id: channel,
                    diff,
                    repo_url: repo,
                    commit_sha: commit,
                    file_path: file,
                    parent_commit_sha: parent_commit,
                    source_branch,
                    target_branch,
                    pr_number: pr,
                    language: lang,
                    description,
                    reply_to,
                },
            )
            .await
        }
        MessagesCmd::Edit { event, content } => cmd_edit_message(client, &event, &content).await,
        MessagesCmd::Delete {
            event,
            action_id,
            reason_code,
            public_reason,
        } => {
            cmd_delete_message(
                client,
                &event,
                action_id,
                reason_code.as_deref(),
                public_reason.as_deref(),
            )
            .await
        }
        MessagesCmd::Get {
            channel,
            limit,
            before,
            since,
            kinds,
        } => {
            cmd_get_messages(
                client,
                &channel,
                limit,
                before,
                since,
                kinds.as_deref(),
                format,
            )
            .await
        }
        MessagesCmd::Thread {
            channel,
            event,
            link,
            limit,
            depth_limit,
        } => {
            let (channel, event, expected_root) =
                match link {
                    Some(link) => {
                        let parsed = crate::links::parse_message_link(&link)?;
                        (parsed.channel_id, parsed.message_id, parsed.thread_root_id)
                    }
                    None => match (channel, event) {
                        (Some(channel), Some(event)) => (channel, event, None),
                        _ => return Err(CliError::Usage(
                            "messages thread requires either --link or both --channel and --event"
                                .into(),
                        )),
                    },
                };
            cmd_get_thread(
                client,
                &channel,
                &event,
                expected_root.as_deref(),
                limit,
                depth_limit,
                format,
            )
            .await
        }
        MessagesCmd::Search {
            query,
            author,
            since,
            limit,
        } => {
            cmd_search(
                client,
                query.as_deref(),
                author.as_deref(),
                since,
                limit,
                format,
            )
            .await
        }
        MessagesCmd::Vote { event, direction } => {
            cmd_vote_on_post(client, &event, &direction).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        channel_id_from_event, cmd_get_thread, cmd_send_message, event_mention_pubkeys,
        find_root_from_tags, format_events, match_profiles_by_name, merge_message_mentions,
        missing_members, normalize_explicit_mentions, parse_member_pubkeys,
        resolve_names_to_pubkeys, resolve_thread_target, thread_ref_from_event,
        thread_ref_from_parent_tags, BuzzClient, CliError, Uuid,
    };
    use buzz_sdk::mentions::{
        extract_at_mentions_with_known, extract_at_names, match_names_to_profiles, MentionProfile,
    };
    use nostr::Keys;
    use serde_json::json;

    const ID_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ID_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const PUBKEY: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    // Three real pubkeys (lowercase 64-char hex) used by parse_member_pubkeys tests.
    // See the test's own comment on what `PublicKey::from_hex` actually validates.
    const PK_VALID_A: &str = "35c18ae273fccfaf80d629e20e7f8721b90499379addff533054acc2504c12b4";
    const PK_VALID_B: &str = "c6237ef84fa537c78dcee78efd2d4e59f728859c7f194da42ac51ededfa0be05";
    const PK_VALID_C: &str = "f4a42a97e594b77bdbd8ee35191c8b28a94a4cb871d96f32921558275421fb68";

    #[test]
    fn compact_event_format_remains_the_three_key_contract() {
        let normalized = serde_json::json!([{
            "id": ID_A,
            "pubkey": PUBKEY,
            "kind": 9,
            "content": "compact content",
            "created_at": 1_787_754_972_u64,
            "tags": [["h", "channel-id"]],
            "sig": "d".repeat(128),
        }])
        .to_string();

        let output: Vec<serde_json::Value> =
            serde_json::from_str(&format_events(&normalized, &crate::OutputFormat::Compact))
                .unwrap();

        assert_eq!(
            output[0],
            serde_json::json!({
                "id": ID_A,
                "content": "compact content",
                "created_at": 1_787_754_972_u64,
            })
        );
    }

    #[tokio::test]
    async fn malformed_channel_is_rejected_before_thread_fetch() {
        let client =
            BuzzClient::new("http://127.0.0.1:1".into(), Keys::generate(), None, None).unwrap();
        let error = cmd_get_thread(
            &client,
            "not-a-uuid",
            ID_A,
            None,
            None,
            None,
            &crate::OutputFormat::Json,
        )
        .await
        .unwrap_err();

        assert!(matches!(error, CliError::Usage(_)));
        assert!(error.to_string().contains("invalid UUID"));
    }

    #[test]
    fn selected_event_derives_authoritative_channel_and_root() {
        let channel = "123e4567-e89b-12d3-a456-426614174000";
        let event = json!({
            "tags": [
                ["h", channel],
                ["e", ID_A, "", "root"],
                ["e", ID_B, "", "reply"],
            ]
        });

        assert_eq!(
            channel_id_from_event(ID_B, &event).unwrap().to_string(),
            channel
        );
        assert_eq!(
            thread_ref_from_event(ID_B, &event)
                .unwrap()
                .root_event_id
                .to_hex(),
            ID_A
        );
    }

    #[test]
    fn selected_event_requires_a_valid_channel_tag() {
        let missing = json!({"tags": []});
        let malformed = json!({"tags": [["h", "not-a-uuid"]]});
        assert!(channel_id_from_event(ID_A, &missing).is_err());
        assert!(channel_id_from_event(ID_A, &malformed).is_err());
    }

    #[test]
    fn thread_target_rejects_wrong_channel_or_root_hint() {
        let channel = "123e4567-e89b-12d3-a456-426614174000";
        let other_channel = "123e4567-e89b-12d3-a456-426614174001";
        let selected = json!({
            "tags": [["h", channel], ["e", ID_A, "", "root"], ["e", ID_B, "", "reply"]]
        });

        assert!(resolve_thread_target(
            Uuid::parse_str(other_channel).unwrap(),
            ID_B,
            Some(ID_A),
            &selected,
        )
        .is_err());
        assert!(resolve_thread_target(
            Uuid::parse_str(channel).unwrap(),
            ID_B,
            Some(ID_B),
            &selected,
        )
        .is_err());
        assert_eq!(
            resolve_thread_target(
                Uuid::parse_str(channel).unwrap(),
                ID_B,
                Some(ID_A),
                &selected,
            )
            .unwrap(),
            ID_A
        );
    }

    #[test]
    fn root_marker_wins_over_reply_marker() {
        let tags = json!([
            ["e", ID_A, "", "root"],
            ["e", ID_B, "", "reply"],
            ["p", PUBKEY],
        ]);
        assert_eq!(find_root_from_tags(&tags).as_deref(), Some(ID_A));
    }

    #[test]
    fn root_marker_without_reply_is_top_level() {
        let tags = json!([["e", ID_A, "", "root"], ["p", PUBKEY],]);
        assert!(find_root_from_tags(&tags).is_none());
    }

    #[test]
    fn root_only_parent_starts_cli_reply_thread_at_parent() {
        let tags = json!([["e", ID_A, "", "root"]]);
        let parent = nostr::EventId::from_hex(ID_B).expect("valid parent id");

        let thread_ref = thread_ref_from_parent_tags(parent, ID_B, &tags).expect("thread ref");

        assert_eq!(thread_ref.parent_event_id, parent);
        assert_eq!(thread_ref.root_event_id, parent);
    }

    #[test]
    fn reply_only_falls_back_to_reply_target() {
        // Direct reply to a top-level message — the parent's only e-tag is a
        // "reply" marker pointing at it; treat the reply target as the root.
        let tags = json!([["e", ID_B, "", "reply"], ["p", PUBKEY],]);
        assert_eq!(find_root_from_tags(&tags).as_deref(), Some(ID_B));
    }

    #[test]
    fn no_thread_markers_returns_none() {
        let tags = json!([["p", PUBKEY], ["h", "channel-uuid"],]);
        assert!(find_root_from_tags(&tags).is_none());
    }

    #[test]
    fn unmarked_e_tag_ignored() {
        // NIP-10 deprecated positional markers; ignore e-tags lacking an
        // explicit "root"/"reply" marker rather than guessing.
        let tags = json!([["e", ID_A], ["e", ID_B, ""],]);
        assert!(find_root_from_tags(&tags).is_none());
    }

    #[test]
    fn malformed_tags_are_skipped_and_root_only_is_top_level() {
        // Invalid entries are ignored, leaving a valid root-only marker; the
        // shared collapse rule still classifies that parent as top-level.
        let tags = json!([
            "not-an-array",
            ["e"],
            ["e", "short"],
            ["e", ID_A, "", "root"],
        ]);
        assert!(find_root_from_tags(&tags).is_none());
    }

    #[test]
    fn malformed_marker_id_is_ignored() {
        // Parent event has a "root" marker whose value isn't a valid 64-hex
        // event id (other-client bug, relay-accepted). Treat the marker as
        // absent so the caller falls back to root == parent rather than
        // failing to send the reply.
        let tags = json!([["e", "not-a-valid-id", "", "root"], ["p", PUBKEY],]);
        assert!(find_root_from_tags(&tags).is_none());
    }

    #[test]
    fn malformed_root_does_not_shadow_valid_reply() {
        // If "root" is malformed but "reply" is valid, fall back to "reply".
        let tags = json!([["e", "garbage", "", "root"], ["e", ID_B, "", "reply"],]);
        assert_eq!(find_root_from_tags(&tags).as_deref(), Some(ID_B));
    }

    #[test]
    fn non_array_input_returns_none() {
        assert!(find_root_from_tags(&json!({})).is_none());
        assert!(find_root_from_tags(&json!(null)).is_none());
    }

    //
    // These tests don't hit the network — they prove that *given* the
    // events the relay returns, the CLI's parse + match wiring produces
    // the right pubkeys. The async I/O wrapper around them is one
    // straight line; the pure stages it composes are exercised here and
    // in buzz-sdk.

    /// End-to-end (sans I/O): body text → extracted names → matched
    /// member pubkeys, using realistic 39002 + kind:0 event JSON.
    /// This is the regression guard for the previous stub that always
    /// returned `vec![]`.
    #[test]
    fn cli_pipeline_resolves_body_at_names_to_member_pubkeys() {
        // kind 39002 channel-members event with three members.
        let members_event = json!({
            "kind": 39002,
            "tags": [
                ["d", "00000000-0000-0000-0000-000000000000"],
                ["p", PK_VALID_A, "", "member"],
                ["p", PK_VALID_B, "", "member"],
                ["p", PK_VALID_C, "", "member"],
            ],
            "content": "",
        });
        assert_eq!(
            parse_member_pubkeys(&members_event),
            vec![PK_VALID_A, PK_VALID_B, PK_VALID_C]
        );

        // Three kind:0 profile events.
        let entries = vec![
            MentionProfile {
                pubkey: PK_VALID_A,
                content_json: r#"{"display_name":"Alice"}"#,
            },
            MentionProfile {
                pubkey: PK_VALID_B,
                content_json: r#"{"display_name":"Bob"}"#,
            },
            MentionProfile {
                pubkey: PK_VALID_C,
                content_json: r#"{"name":"Carol"}"#,
            },
        ];

        // Body mentions Alice and Carol (display_name fallback to `name`).
        let names = extract_at_names("hello @alice and @CAROL");
        let resolved = match_names_to_profiles(&names, &entries);
        assert_eq!(resolved, vec![PK_VALID_A, PK_VALID_C]);
    }

    #[test]
    fn cli_pipeline_resolves_multiword_display_names() {
        let profile_events: Vec<serde_json::Value> = vec![
            json!({
                "pubkey": PK_VALID_A,
                "content": r#"{"display_name":"Will Pfleger"}"#,
            }),
            json!({
                "pubkey": PK_VALID_B,
                "content": r#"{"display_name":"Alice"}"#,
            }),
        ];

        // Simulate the single-parse pipeline from resolve_content_mentions.
        let mut name_to_pubkeys: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let mut display_names: Vec<String> = Vec::new();
        for e in &profile_events {
            let pubkey = e.get("pubkey").unwrap().as_str().unwrap();
            let content_json = e.get("content").unwrap().as_str().unwrap();
            let v: serde_json::Value = serde_json::from_str(content_json).unwrap();
            let name = v
                .get("display_name")
                .or_else(|| v.get("name"))
                .and_then(|n| n.as_str())
                .filter(|n| !n.is_empty())
                .unwrap();
            let lower = name.to_ascii_lowercase();
            name_to_pubkeys
                .entry(lower)
                .or_default()
                .push(pubkey.to_string());
            display_names.push(name.to_string());
        }

        let known_refs: Vec<&str> = display_names.iter().map(|s| s.as_str()).collect();
        let names = extract_at_mentions_with_known("hey @Will Pfleger and @alice!", &known_refs);
        assert_eq!(names, vec!["will pfleger", "alice"]);

        let resolved: Vec<String> = names
            .iter()
            .flat_map(|n| name_to_pubkeys.get(n).into_iter().flatten())
            .cloned()
            .collect();
        assert_eq!(resolved, vec![PK_VALID_A, PK_VALID_B]);
    }

    #[test]
    fn cli_pipeline_returns_empty_when_no_at_names() {
        // Sanity: no `@names` in body → no profile match attempt needed.
        let names = extract_at_names("plain message, no mentions");
        assert!(names.is_empty());
    }

    #[test]
    fn parse_member_pubkeys_ignores_non_p_tags() {
        let event = json!({
            "tags": [
                ["d", "channel-id"],
                ["p", PK_VALID_A],
                ["h", "channel-id"],
                ["e", "some-event"],
                ["p", PK_VALID_B, "wss://relay", "member"],
            ],
        });
        assert_eq!(parse_member_pubkeys(&event), vec![PK_VALID_A, PK_VALID_B]);
    }

    #[test]
    fn parse_member_pubkeys_handles_malformed_event() {
        assert!(parse_member_pubkeys(&json!({})).is_empty());
        assert!(parse_member_pubkeys(&json!({"tags": "not an array"})).is_empty());
        assert!(parse_member_pubkeys(&json!({"tags": [["p"]]})).is_empty());
    }

    #[test]
    fn parse_member_pubkeys_filters_invalid_hex() {
        // `PublicKey::from_hex` rejects non-hex and wrong-length inputs and
        // canonicalizes hex case. (Note: it accepts any 64-char x-only hex
        // whose integer value is in field; it does not verify the point is
        // actually on the curve — same as MCP's behavior.)
        let pk_uppercase: String = PK_VALID_A.to_ascii_uppercase();
        let event = json!({
            "tags": [
                ["p", PK_VALID_A],       // valid, lowercase
                ["p", pk_uppercase],     // valid hex, canonicalized to lowercase
                ["p", "too-short"],      // length fail
                ["p", "z".repeat(64)],   // non-hex chars
                ["p", "a".repeat(63)],   // off-by-one length
            ],
        });
        assert_eq!(parse_member_pubkeys(&event), vec![PK_VALID_A, PK_VALID_A]);
    }

    #[test]
    fn explicit_mentions_accept_hex_and_npub_and_deduplicate() {
        use nostr::ToBech32;
        let npub = nostr::PublicKey::from_hex(PK_VALID_A)
            .unwrap()
            .to_bech32()
            .unwrap();
        assert_eq!(
            normalize_explicit_mentions(&[PK_VALID_A.into(), npub]).unwrap(),
            vec![PK_VALID_A]
        );
        assert!(normalize_explicit_mentions(&["not-a-key".into()]).is_err());
    }

    #[test]
    fn explicit_mentions_authorize_presentation_text_without_name_resolution() {
        let names = vec!["renamed user".into()];
        let profiles = std::collections::HashMap::new();
        assert_eq!(
            resolve_names_to_pubkeys(&names, &profiles, true).unwrap(),
            Vec::<String>::new()
        );
        assert!(resolve_names_to_pubkeys(&names, &profiles, false).is_err());
    }

    #[test]
    fn explicit_mentions_authorize_ambiguous_presentation_text() {
        let names = vec!["alice".into()];
        let profiles = std::collections::HashMap::from([(
            "alice".into(),
            vec![PK_VALID_A.into(), PK_VALID_B.into()],
        )]);
        assert_eq!(
            resolve_names_to_pubkeys(&names, &profiles, true).unwrap(),
            Vec::<String>::new()
        );
        let error = resolve_names_to_pubkeys(&names, &profiles, false).unwrap_err();
        assert!(error.to_string().contains(PK_VALID_A));
        assert!(error.to_string().contains(PK_VALID_B));
    }

    #[test]
    fn explicit_mentions_make_all_at_names_presentation_only() {
        let names = vec!["alice".into(), "bob".into()];
        let profiles = std::collections::HashMap::from([("alice".into(), vec![PK_VALID_A.into()])]);
        assert_eq!(
            resolve_names_to_pubkeys(&names, &profiles, true).unwrap(),
            vec![PK_VALID_A]
        );
        assert!(resolve_names_to_pubkeys(&names, &profiles, false).is_err());
    }

    #[test]
    fn combined_mention_union_errors_instead_of_truncating() {
        let explicit: Vec<String> = (0..50).map(|i| format!("explicit-{i}")).collect();
        assert!(merge_message_mentions(&explicit, &[], &["resolved-bob".into()]).is_err());

        let mut with_duplicate = explicit.clone();
        with_duplicate.push(explicit[0].clone());
        assert_eq!(
            merge_message_mentions(&with_duplicate, &[explicit[1].clone()], &[])
                .unwrap()
                .len(),
            50
        );
    }

    #[test]
    fn membership_preflight_lists_only_missing_mentions() {
        assert_eq!(
            missing_members(
                &[PK_VALID_A.into(), PK_VALID_B.into()],
                &[PK_VALID_A.into()]
            ),
            vec![PK_VALID_B]
        );
    }

    #[test]
    fn mention_evidence_comes_from_signed_event_tags() {
        use nostr::{EventBuilder, Keys, Tag};
        let event = EventBuilder::text_note("hello")
            .tags(vec![Tag::parse(["p", PK_VALID_A]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert_eq!(event_mention_pubkeys(&event), vec![PK_VALID_A]);
    }

    // ---- match_profiles_by_name (author resolution for `messages search --author`) ----

    fn profile_event(
        pubkey: &str,
        display_name: Option<&str>,
        name: Option<&str>,
    ) -> serde_json::Value {
        let mut content = serde_json::Map::new();
        if let Some(d) = display_name {
            content.insert("display_name".into(), json!(d));
        }
        if let Some(n) = name {
            content.insert("name".into(), json!(n));
        }
        json!({
            "pubkey": pubkey,
            "content": serde_json::Value::Object(content).to_string(),
        })
    }

    #[test]
    fn author_name_match_is_exact_case_insensitive() {
        let events = vec![
            profile_event(PK_VALID_A, Some("Aaron"), Some("aaron")),
            // Substring only — NIP-50 may return it, but it must not match.
            profile_event(PK_VALID_B, Some("Aaronson"), None),
        ];
        let matches = match_profiles_by_name(&events, "aArOn");
        assert_eq!(matches, vec![(PK_VALID_A.to_string(), "Aaron".to_string())]);
    }

    #[test]
    fn author_name_ambiguity_returns_all_candidates() {
        let events = vec![
            profile_event(PK_VALID_A, Some("Sam"), None),
            profile_event(PK_VALID_B, None, Some("sam")),
        ];
        let matches = match_profiles_by_name(&events, "sam");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn author_name_no_match_and_malformed_content() {
        let events = vec![
            profile_event(PK_VALID_A, Some("Aaron"), None),
            json!({"pubkey": PK_VALID_B, "content": "not-json"}),
            json!({"content": "{}"}), // missing pubkey
        ];
        assert!(match_profiles_by_name(&events, "Zoe").is_empty());
    }

    #[test]
    fn author_name_dedups_replaceable_event_copies() {
        // Same (pubkey, name) appearing twice (e.g. duplicate kind:0 rows)
        // must resolve unambiguously.
        let events = vec![
            profile_event(PK_VALID_A, Some("Aaron"), None),
            profile_event(PK_VALID_A, Some("Aaron"), None),
        ];
        assert_eq!(match_profiles_by_name(&events, "Aaron").len(), 1);
    }

    // ── cmd_send_message — emoji-tag binding seam ─────────────────────────
    //
    // These tests drive `cmd_send_message` through a minimal fake relay
    // serving `/query` (emoji palette) and `/events` (event submission).
    //
    // Content with no `@` and no explicit mentions bypasses member-resolution
    // relay calls, so the only relay traffic is:
    //   1. POST /query  — emoji palette fetch (when content has `:`)
    //   2. POST /events — signed event submission
    //
    // Removing the resolver call at messages.rs:687-691 or passing &[] at
    // :718 would cause the emoji-tag assertions below to fail.

    use axum::body::Bytes as AxumBytes;
    use axum::extract::State as AxumState;
    use axum::http::{HeaderMap as AxumHeaderMap, StatusCode as AxumStatusCode};
    use axum::routing::post as axum_post;
    use axum::Router as AxumRouter;
    use std::net::SocketAddr as StdSocketAddr;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc as StdArc;
    use tokio::net::TcpListener as TokioTcpListener;

    /// Captured body of a POST /events call.
    #[derive(Clone, Default)]
    struct CapturedEvent {
        body: String,
    }

    /// Minimal fake relay for send-path tests.
    ///
    /// - `/query` returns the given `query_body` on every call and increments
    ///   `query_count`.
    /// - `/events` returns `{"event_id":"fake","accepted":true}` and records
    ///   the raw event JSON in `captured_event`.
    async fn fake_send_relay(
        query_body: String,
    ) -> (
        String,
        StdArc<AtomicU32>,
        StdArc<std::sync::Mutex<Option<CapturedEvent>>>,
    ) {
        let query_count = StdArc::new(AtomicU32::new(0));
        let captured_event: StdArc<std::sync::Mutex<Option<CapturedEvent>>> =
            StdArc::new(std::sync::Mutex::new(None));

        type S = (
            StdArc<AtomicU32>,
            String,
            StdArc<std::sync::Mutex<Option<CapturedEvent>>>,
        );
        let state: S = (query_count.clone(), query_body, captured_event.clone());

        let app = AxumRouter::new()
            .route(
                "/query",
                axum_post(
                    |AxumState((count, body, _)): AxumState<S>,
                     _headers: AxumHeaderMap,
                     _req: AxumBytes| async move {
                        count.fetch_add(1, Ordering::Relaxed);
                        (
                            AxumStatusCode::OK,
                            [("content-type", "application/json")],
                            body,
                        )
                    },
                ),
            )
            .route(
                "/events",
                axum_post(
                    |AxumState((_, _, cap)): AxumState<S>,
                     _headers: AxumHeaderMap,
                     body: AxumBytes| async move {
                        let body_str = String::from_utf8_lossy(&body).to_string();
                        *cap.lock().unwrap() = Some(CapturedEvent { body: body_str });
                        (
                            AxumStatusCode::OK,
                            [("content-type", "application/json")],
                            r#"{"event_id":"fake0000","accepted":true}"#,
                        )
                    },
                ),
            )
            .with_state(state);

        let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: StdSocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), query_count, captured_event)
    }

    /// Palette JSON with one emoji: `wave` → some URL.
    fn send_palette_response() -> String {
        serde_json::json!([{
            "created_at": 100,
            "tags": [
                ["d", "buzz:custom-emoji"],
                ["emoji", "wave", "https://cdn.example.com/wave.png"],
                ["emoji", "sweatblob", "https://cdn.example.com/sweatblob.gif"]
            ]
        }])
        .to_string()
    }

    /// A valid channel UUID used across send-path tests.
    const SEND_TEST_CHANNEL: &str = "123e4567-e89b-12d3-a456-426614174000";

    fn send_params(content: &str) -> super::SendMessageParams {
        super::SendMessageParams {
            channel_id: SEND_TEST_CHANNEL.to_string(),
            content: content.to_string(),
            kind: None,
            reply_to: None,
            broadcast: false,
            files: vec![],
            mentions: vec![],
            cc: vec![],
        }
    }

    #[tokio::test]
    async fn cmd_send_message_attaches_emoji_tags_for_known_shortcodes() {
        // Content contains `:wave:` which resolves in the palette.
        // The submitted event must carry an `emoji` tag for `wave`.
        let (url, query_count, captured_event) = fake_send_relay(send_palette_response()).await;
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();

        cmd_send_message(&client, send_params("hello :wave: everyone"))
            .await
            .unwrap();

        // Palette was queried at least once (short-circuit was NOT triggered).
        assert!(
            query_count.load(Ordering::Relaxed) >= 1,
            "palette must be queried when content has a colon"
        );

        // Submitted event must contain an emoji tag for `wave`.
        let raw = captured_event.lock().unwrap();
        let raw = raw.as_ref().expect("event must have been submitted");
        let event: serde_json::Value = serde_json::from_str(&raw.body).unwrap();
        let tags: Vec<Vec<String>> = event["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                t.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            })
            .collect();
        let emoji_tags: Vec<&Vec<String>> = tags
            .iter()
            .filter(|t| t.first().map(|s| s.as_str()) == Some("emoji"))
            .collect();
        assert!(
            emoji_tags
                .iter()
                .any(|t| t.get(1).map(|s| s.as_str()) == Some("wave")),
            "submitted event must have an emoji tag for `wave`, got tags: {tags:?}"
        );
        // Unknown shortcodes must not produce tags.
        assert!(
            !emoji_tags
                .iter()
                .any(|t| t.get(1).map(|s| s.as_str()) == Some("notreal")),
            "unknown shortcodes must not produce emoji tags"
        );
    }

    #[tokio::test]
    async fn cmd_send_message_skips_palette_query_when_no_colon_in_content() {
        // Content has no `:` at all — the palette query must be skipped
        // entirely (zero RTTs), and the submitted event must have no emoji tags.
        let (url, query_count, captured_event) = fake_send_relay(send_palette_response()).await;
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();

        cmd_send_message(&client, send_params("plain message no colons"))
            .await
            .unwrap();

        assert_eq!(
            query_count.load(Ordering::Relaxed),
            0,
            "palette must NOT be queried when content has no colon"
        );

        // Submitted event must have no emoji tags.
        let raw = captured_event.lock().unwrap();
        let raw = raw.as_ref().expect("event must have been submitted");
        let event: serde_json::Value = serde_json::from_str(&raw.body).unwrap();
        let tags: Vec<Vec<String>> = event["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                t.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            })
            .collect();
        let emoji_tags: Vec<&Vec<String>> = tags
            .iter()
            .filter(|t| t.first().map(|s| s.as_str()) == Some("emoji"))
            .collect();
        assert!(
            emoji_tags.is_empty(),
            "no-colon content must produce no emoji tags, got: {emoji_tags:?}"
        );
    }

    #[tokio::test]
    async fn cmd_send_message_succeeds_when_palette_query_errors() {
        // Palette enrichment is decorative — a 500 from the `/query` endpoint
        // must not abort delivery; the message must still be sent with zero
        // emoji tags, and a diagnostic must be emitted to stderr.

        // Fake relay: `/query` returns 500, `/events` accepts and captures.
        let captured_event: StdArc<std::sync::Mutex<Option<CapturedEvent>>> =
            StdArc::new(std::sync::Mutex::new(None));
        let cap = captured_event.clone();
        let app = AxumRouter::new()
            .route(
                "/query",
                axum_post(|_headers: AxumHeaderMap, _req: AxumBytes| async move {
                    (
                        AxumStatusCode::INTERNAL_SERVER_ERROR,
                        [("content-type", "application/json")],
                        r#"{"error":"unavailable"}"#,
                    )
                }),
            )
            .route(
                "/events",
                axum_post(move |_headers: AxumHeaderMap, body: AxumBytes| {
                    let cap = cap.clone();
                    async move {
                        let body_str = String::from_utf8_lossy(&body).to_string();
                        *cap.lock().unwrap() = Some(CapturedEvent { body: body_str });
                        (
                            AxumStatusCode::OK,
                            [("content-type", "application/json")],
                            r#"{"event_id":"fake0001","accepted":true}"#,
                        )
                    }
                }),
            );

        let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: StdSocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("http://{addr}");
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();

        // Must not return Err — a palette failure is a soft warning.
        cmd_send_message(&client, send_params(":wave: message with emoji candidate"))
            .await
            .expect("send must succeed even when palette query returns 500");

        // Submitted event must have zero emoji tags (fallback to empty).
        let raw = captured_event.lock().unwrap();
        let raw = raw.as_ref().expect("event must have been submitted");
        let event: serde_json::Value = serde_json::from_str(&raw.body).unwrap();
        let tags: Vec<Vec<String>> = event["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                t.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            })
            .collect();
        let emoji_tags: Vec<&Vec<String>> = tags
            .iter()
            .filter(|t| t.first().map(|s| s.as_str()) == Some("emoji"))
            .collect();
        assert!(
            emoji_tags.is_empty(),
            "palette-error fallback must produce no emoji tags, got: {emoji_tags:?}"
        );
    }
}

#[cfg(test)]
mod manager_copy_tests {
    use super::manager_copy_for_crossing;

    const BOSS: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PEER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const OUTSIDER: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const OWNER: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

    /// These tests set process-wide environment, so they take a lock rather
    /// than racing each other through the same two variables.
    fn with_line<T>(manager: Option<&str>, house: &[&str], body: impl FnOnce() -> T) -> T {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = LOCK.get_or_init(|| Mutex::new(())).lock();
        // A poisoned lock means another test panicked mid-setup; the variables
        // are reset below either way, so the value is still usable.
        let _guard = guard.unwrap_or_else(|poisoned| poisoned.into_inner());
        // SAFETY: the lock serialises every writer in this module, and the
        // binary is single-threaded at this point in the test harness.
        unsafe {
            match manager {
                Some(value) => std::env::set_var("BUZZ_ACP_MANAGER", value),
                None => std::env::remove_var("BUZZ_ACP_MANAGER"),
            }
            std::env::set_var("BUZZ_ACP_SAME_HOUSE", house.join(","));
        }
        let outcome = body();
        unsafe {
            std::env::remove_var("BUZZ_ACP_MANAGER");
            std::env::remove_var("BUZZ_ACP_SAME_HOUSE");
        }
        outcome
    }

    /// The case the whole feature exists for, and the one a model kept
    /// forgetting: reaching into another department informs your manager.
    #[test]
    fn leaving_your_department_copies_your_manager() {
        with_line(Some(BOSS), &[BOSS, PEER], || {
            assert_eq!(
                manager_copy_for_crossing(&[OUTSIDER.to_string()], &[], Some(OWNER)),
                Some(BOSS.to_string())
            );
        });
    }

    /// Two people in one department working together is the department doing
    /// its job, and a copy on each such message would bury the manager in it.
    #[test]
    fn talking_to_a_peer_copies_nobody() {
        with_line(Some(BOSS), &[BOSS, PEER], || {
            assert_eq!(
                manager_copy_for_crossing(&[PEER.to_string()], &[], Some(OWNER)),
                None
            );
        });
    }

    /// Copying your manager on a message addressed to your manager informs
    /// the same person twice.
    #[test]
    fn talking_to_your_manager_copies_nobody() {
        with_line(Some(BOSS), &[BOSS, PEER], || {
            assert_eq!(
                manager_copy_for_crossing(&[BOSS.to_string()], &[], Some(OWNER)),
                None
            );
        });
    }

    /// Talking to the person who owns the whole chart informs nobody further
    /// up, because there is nobody further up.
    #[test]
    fn talking_to_the_owner_copies_nobody() {
        with_line(Some(BOSS), &[BOSS], || {
            assert_eq!(
                manager_copy_for_crossing(&[OWNER.to_string()], &[], Some(OWNER)),
                None
            );
        });
    }

    /// One crossing in a mixed message is still a crossing.
    #[test]
    fn one_outsider_among_peers_is_enough() {
        with_line(Some(BOSS), &[BOSS, PEER], || {
            assert_eq!(
                manager_copy_for_crossing(
                    &[PEER.to_string(), OUTSIDER.to_string()],
                    &[],
                    Some(OWNER)
                ),
                Some(BOSS.to_string())
            );
        });
    }

    /// An agent that already copied its manager by hand is not copied twice.
    #[test]
    fn an_explicit_copy_is_not_duplicated() {
        with_line(Some(BOSS), &[BOSS], || {
            assert_eq!(
                manager_copy_for_crossing(
                    &[OUTSIDER.to_string()],
                    &[BOSS.to_uppercase()],
                    Some(OWNER)
                ),
                None
            );
        });
    }

    /// Nobody to inform: an agent at the top of the chart reports to the
    /// owner, and the policy adds nothing rather than inventing a recipient.
    #[test]
    fn an_agent_with_no_manager_copies_nobody() {
        with_line(None, &[], || {
            assert_eq!(
                manager_copy_for_crossing(&[OUTSIDER.to_string()], &[], Some(OWNER)),
                None
            );
        });
    }

    /// A message addressed to nobody crosses nothing.
    #[test]
    fn a_message_with_no_mentions_copies_nobody() {
        with_line(Some(BOSS), &[BOSS, PEER], || {
            assert_eq!(manager_copy_for_crossing(&[], &[], Some(OWNER)), None);
        });
    }

    /// Case is not identity, in either direction.
    #[test]
    fn the_house_is_matched_without_regard_to_case() {
        with_line(Some(BOSS), &[&PEER.to_uppercase()], || {
            assert_eq!(
                manager_copy_for_crossing(&[PEER.to_string()], &[], Some(OWNER)),
                None
            );
        });
    }
}

#[cfg(test)]
mod send_receipt_tests {
    use super::record_send_receipt;

    /// Tests share process environment, so they serialise through one lock.
    fn with_receipt<T>(path: Option<&std::path::Path>, body: impl FnOnce() -> T) -> T {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = LOCK.get_or_init(|| Mutex::new(())).lock();
        let _guard = guard.unwrap_or_else(|poisoned| poisoned.into_inner());
        // SAFETY: the lock serialises every writer here and the test harness
        // is single-threaded at this point.
        unsafe {
            match path {
                Some(p) => std::env::set_var("BUZZ_SEND_RECEIPT", p),
                None => std::env::remove_var("BUZZ_SEND_RECEIPT"),
            }
        }
        let outcome = body();
        unsafe { std::env::remove_var("BUZZ_SEND_RECEIPT") }
        outcome
    }

    /// The receipt is the only evidence the harness has that a turn actually
    /// said something, so a successful send must leave one.
    #[test]
    fn a_send_leaves_a_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("receipt");
        with_receipt(Some(&path), record_send_receipt);
        assert!(path.is_file(), "the receipt should exist at {path:?}");
    }

    /// Two sends in one turn must still read as "something was sent", and the
    /// file must not grow without end across a long-running agent.
    #[test]
    fn a_second_send_replaces_rather_than_appends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt");
        with_receipt(Some(&path), || {
            record_send_receipt();
            record_send_receipt();
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"sent\n");
    }

    /// Nothing configured is not an error: an agent launched outside the app
    /// still sends messages, it simply has nowhere to record them.
    #[test]
    fn no_configured_path_is_not_a_failure() {
        with_receipt(None, record_send_receipt);
    }

    /// An unwritable path costs a notice the owner might not have needed.
    /// Failing the send over it would cost the message, which is the thing
    /// that actually mattered.
    #[test]
    fn an_unwritable_path_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        // A file cannot also be a directory, so creating the parent fails.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        with_receipt(Some(&blocker.join("receipt")), record_send_receipt);
    }
}

#[cfg(test)]
mod addressee_tests {
    use super::*;

    fn team() -> Vec<(String, String)> {
        vec![
            ("cortex".into(), "CORTEX".into()),
            ("pilot".into(), "Pilot Project Manager".into()),
            ("signal".into(), "Signal Instrumentation Engineer".into()),
        ]
    }

    #[test]
    fn a_message_that_opens_with_a_name_addresses_it() {
        assert_eq!(
            opening_addressee("CORTEX، التقرير جاهز").as_deref(),
            Some("CORTEX")
        );
        assert_eq!(
            opening_addressee("Pilot: status?").as_deref(),
            Some("Pilot")
        );
        assert_eq!(
            opening_addressee("يا كورتيكس، شف هذا").as_deref(),
            Some("كورتيكس")
        );
        assert_eq!(
            opening_addressee("\n  Pilot, go ahead").as_deref(),
            Some("Pilot")
        );
    }

    #[test]
    fn a_name_said_in_passing_wakes_no_one() {
        let team = team();
        let woken = |content: &str| {
            opening_addressee(content).and_then(|name| resolve_addressee(&name, &team, "me"))
        };
        assert_eq!(woken("As Pilot noted, the sizing is done"), None);
        assert_eq!(woken("The report is ready"), None);
        assert_eq!(
            woken("The pilot plant sizing we discussed earlier: done"),
            None,
            "a sentence is not a name"
        );
        assert_eq!(woken("Pilot, go ahead").as_deref(), Some("pilot"));
    }

    #[test]
    fn english_arabic_and_full_names_reach_the_same_teammate() {
        let team = team();
        for called in ["CORTEX", "cortex", "كورتيكس", "كورتكس"] {
            assert_eq!(
                resolve_addressee(called, &team, "me").as_deref(),
                Some("cortex"),
                "{called}"
            );
        }
        assert_eq!(
            resolve_addressee("Pilot", &team, "me").as_deref(),
            Some("pilot")
        );
        assert_eq!(
            resolve_addressee("بايلوت", &team, "me").as_deref(),
            Some("pilot")
        );
        assert_eq!(
            resolve_addressee("Signal Instrumentation Engineer", &team, "me").as_deref(),
            Some("signal")
        );
        assert_eq!(resolve_addressee("Note", &team, "me"), None);
    }

    #[test]
    fn nobody_is_woken_by_their_own_message_or_by_a_guess() {
        let team = team();
        assert_eq!(resolve_addressee("CORTEX", &team, "cortex"), None);
        let twins = vec![
            ("a".to_string(), "Pilot One".to_string()),
            ("b".to_string(), "Pilot Two".to_string()),
        ];
        assert_eq!(resolve_addressee("Pilot", &twins, "me"), None);
    }

    #[test]
    fn spelling_variants_compare_equal() {
        assert_eq!(normalize_name("أطلس"), normalize_name("اطلس"));
        assert_eq!(normalize_name("CORTEX"), "cortex");
        assert_eq!(normalize_name("كورتـكس"), normalize_name("كورتكس"));
    }
}
