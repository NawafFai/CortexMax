//! Owner-reviewed agent draft requests published through Buzz observer frames.

use buzz_core::observer::{encrypt_observer_payload, OBSERVER_FRAME_TELEMETRY};
use nostr::{Event, Keys, PublicKey};
use serde::Serialize;

use crate::error::CliError;

const AGENT_REQUEST_KIND: &str = "agent_management_request";
const PROJECT_CHANNEL_REQUEST_KIND: &str = "project_channel_request";
const FOLDER_PROJECT_REQUEST_KIND: &str = "folder_project_request";
const MAX_NAME_CHARS: usize = 120;
const MAX_PROMPT_CHARS: usize = 20_000;
/// The desktop's own bound on a project root.
const MAX_FOLDER_CHARS: usize = 4_096;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentDraft {
    pub channel_id: String,
    pub display_name: String,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentDraft {
    pub channel_id: String,
    pub agent_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectChannelDraft {
    pub home_channel_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub visibility: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_name: Option<String>,
}

/// A project the agent asks its owner to start: a channel bound to a folder on
/// the owner's machine, created with its management room and handover note.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderProjectDraft {
    /// The channel the request comes from, where the owner's app reports back.
    pub channel_id: String,
    pub name: String,
    /// Absolute, canonical path of the folder the project is bound to.
    pub folder: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub visibility: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagementRequest<T> {
    #[serde(rename = "type")]
    request_type: &'static str,
    action: &'static str,
    request_id: String,
    request: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObserverEvent<T> {
    seq: u64,
    timestamp: String,
    kind: &'static str,
    agent_index: Option<usize>,
    channel_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    payload: ManagementRequest<T>,
}

#[derive(Debug)]
pub struct BuiltDraftRequest {
    pub event: Event,
    pub request_id: String,
    pub action: &'static str,
}

fn required(value: String, label: &str, max: usize) -> Result<String, CliError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(CliError::Usage(format!("{label} is required")));
    }
    if value.chars().count() > max {
        return Err(CliError::Usage(format!(
            "{label} is too long (max {max} characters)"
        )));
    }
    Ok(value.to_owned())
}

fn optional(value: Option<String>, label: &str) -> Result<Option<String>, CliError> {
    value.map(|value| required(value, label, 300)).transpose()
}

fn build<T: Serialize>(
    keys: &Keys,
    owner: &PublicKey,
    channel_id: String,
    request_kind: &'static str,
    action: &'static str,
    request: T,
) -> Result<BuiltDraftRequest, CliError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let payload = ObserverEvent {
        seq: 0,
        timestamp: chrono::Utc::now().to_rfc3339(),
        kind: request_kind,
        agent_index: None,
        channel_id: Some(channel_id),
        session_id: None,
        turn_id: None,
        payload: ManagementRequest {
            request_type: request_kind,
            action,
            request_id: request_id.clone(),
            request,
        },
    };
    let encrypted = encrypt_observer_payload(keys, owner, &payload)
        .map_err(|error| CliError::Other(format!("could not encrypt draft request: {error}")))?;
    let event = buzz_sdk::build_agent_observer_frame(
        &owner.to_hex(),
        &keys.public_key().to_hex(),
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    )
    .map_err(|error| CliError::Other(format!("could not build draft request: {error}")))?
    .sign_with_keys(keys)
    .map_err(|error| CliError::Other(format!("could not sign draft request: {error}")))?;
    Ok(BuiltDraftRequest {
        event,
        request_id,
        action,
    })
}

pub fn build_create(
    keys: &Keys,
    owner: &PublicKey,
    draft: CreateAgentDraft,
) -> Result<BuiltDraftRequest, CliError> {
    let channel_id = required(draft.channel_id, "channel", 128)?;
    uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel_id}")))?;
    let request = CreateAgentDraft {
        channel_id: channel_id.clone(),
        display_name: required(draft.display_name, "display name", MAX_NAME_CHARS)?,
        system_prompt: required(draft.system_prompt, "system prompt", MAX_PROMPT_CHARS)?,
    };
    build(
        keys,
        owner,
        channel_id,
        AGENT_REQUEST_KIND,
        "create",
        request,
    )
}

pub fn build_update(
    keys: &Keys,
    owner: &PublicKey,
    draft: UpdateAgentDraft,
) -> Result<BuiltDraftRequest, CliError> {
    let channel_id = required(draft.channel_id, "channel", 128)?;
    uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel_id}")))?;
    let respond_to = optional(draft.respond_to, "respond-to")?;
    if respond_to
        .as_deref()
        .is_some_and(|value| value != "owner-only" && value != "anyone")
    {
        return Err(CliError::Usage(
            "respond-to must be owner-only or anyone".into(),
        ));
    }
    let request = UpdateAgentDraft {
        channel_id: channel_id.clone(),
        agent_name: required(draft.agent_name, "agent name", MAX_NAME_CHARS)?,
        display_name: optional(draft.display_name, "display name")?,
        system_prompt: draft
            .system_prompt
            .map(|value| required(value, "system prompt", MAX_PROMPT_CHARS))
            .transpose()?,
        runtime: optional(draft.runtime, "runtime")?,
        provider: optional(draft.provider, "provider")?,
        model: optional(draft.model, "model")?,
        respond_to,
    };
    if request.display_name.is_none()
        && request.system_prompt.is_none()
        && request.runtime.is_none()
        && request.provider.is_none()
        && request.model.is_none()
        && request.respond_to.is_none()
    {
        return Err(CliError::Usage(
            "include at least one field to update".into(),
        ));
    }
    build(
        keys,
        owner,
        channel_id,
        AGENT_REQUEST_KIND,
        "update",
        request,
    )
}

pub fn build_project_channel(
    keys: &Keys,
    owner: &PublicKey,
    draft: CreateProjectChannelDraft,
) -> Result<BuiltDraftRequest, CliError> {
    let home_channel_id = required(draft.home_channel_id, "home channel", 128)?;
    uuid::Uuid::parse_str(&home_channel_id)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {home_channel_id}")))?;
    let visibility = required(draft.visibility, "visibility", 16)?;
    if visibility != "open" && visibility != "private" {
        return Err(CliError::Usage("visibility must be open or private".into()));
    }
    if draft.ttl_seconds == Some(0) {
        return Err(CliError::Usage("ttl must be greater than zero".into()));
    }
    let request = CreateProjectChannelDraft {
        home_channel_id: home_channel_id.clone(),
        name: required(draft.name, "name", MAX_NAME_CHARS)?,
        description: draft
            .description
            .map(|value| required(value, "description", 2_048))
            .transpose()?,
        visibility,
        ttl_seconds: draft.ttl_seconds,
        template_name: optional(draft.template_name, "template")?,
    };
    build(
        keys,
        owner,
        home_channel_id,
        PROJECT_CHANNEL_REQUEST_KIND,
        "create",
        request,
    )
}

/// Ask the owner's app to start a folder-bound project.
///
/// The owner confirms it in the app; nothing exists until then. The folder is
/// expected already resolved by [`resolve_project_folder`], and is checked
/// again here only for shape.
pub fn build_folder_project(
    keys: &Keys,
    owner: &PublicKey,
    draft: FolderProjectDraft,
) -> Result<BuiltDraftRequest, CliError> {
    let channel_id = required(draft.channel_id, "channel", 128)?;
    uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel_id}")))?;
    let visibility = required(draft.visibility, "visibility", 16)?;
    if visibility != "open" && visibility != "private" {
        return Err(CliError::Usage("visibility must be open or private".into()));
    }
    let folder = required(draft.folder, "folder", MAX_FOLDER_CHARS)?;
    if !std::path::Path::new(&folder).is_absolute() {
        return Err(CliError::Usage("folder must be an absolute path".into()));
    }
    let request = FolderProjectDraft {
        channel_id: channel_id.clone(),
        name: required(draft.name, "name", MAX_NAME_CHARS)?,
        folder,
        description: draft
            .description
            .map(|value| required(value, "description", 2_048))
            .transpose()?,
        visibility,
    };
    build(
        keys,
        owner,
        channel_id,
        FOLDER_PROJECT_REQUEST_KIND,
        "create",
        request,
    )
}

/// The folder as the owner's app will bind it: `~` expanded, symlinks
/// resolved, and required to be an existing directory.
///
/// The CLI runs on the owner's machine, so this is checked here, where a typo
/// fails in the agent's own turn with a reason it can act on, rather than in
/// the owner's confirmation dialog.
pub fn resolve_project_folder(raw: &str) -> Result<String, CliError> {
    let raw = raw.trim();
    let home = || {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| CliError::Usage("HOME is not set, so ~ cannot be expanded".into()))
    };
    let path = match raw.strip_prefix("~/") {
        Some(rest) => home()?.join(rest),
        None if raw == "~" => home()?,
        None => std::path::PathBuf::from(raw),
    };
    if !path.is_absolute() {
        return Err(CliError::Usage(
            "folder must be an absolute path, or start with ~/".into(),
        ));
    }
    let canonical = path.canonicalize().map_err(|error| {
        CliError::Usage(format!("folder not found: {} ({error})", path.display()))
    })?;
    if !canonical.is_dir() {
        return Err(CliError::Usage(format!(
            "not a folder: {}",
            canonical.display()
        )));
    }
    canonical
        .into_os_string()
        .into_string()
        .map_err(|_| CliError::Usage("folder path is not valid UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::observer::{decrypt_observer_payload, OBSERVER_AGENT_TAG, OBSERVER_FRAME_TAG};

    const CHANNEL: &str = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

    #[test]
    fn create_is_owner_encrypted_and_matches_desktop_contract() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let built = build_create(
            &agent,
            &owner.public_key(),
            CreateAgentDraft {
                channel_id: CHANNEL.into(),
                display_name: "Research helper".into(),
                system_prompt: "Find sources.".into(),
            },
        )
        .unwrap();

        assert_eq!(built.event.kind.as_u16(), 24_200);
        let tags: Vec<Vec<String>> = built
            .event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        assert!(tags
            .iter()
            .any(|tag| tag == &["p", &owner.public_key().to_hex()]));
        assert!(tags
            .iter()
            .any(|tag| tag == &[OBSERVER_AGENT_TAG, &agent.public_key().to_hex()]));
        assert!(tags
            .iter()
            .any(|tag| tag == &[OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY]));
        assert!(!tags
            .iter()
            .any(|tag| tag.first().map(String::as_str) == Some("h")));

        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        assert_eq!(payload["kind"], AGENT_REQUEST_KIND);
        assert_eq!(payload["channelId"], CHANNEL);
        assert_eq!(payload["payload"]["type"], AGENT_REQUEST_KIND);
        assert_eq!(payload["payload"]["action"], "create");
        assert_eq!(
            payload["payload"]["request"]["displayName"],
            "Research helper"
        );
        assert!(payload["payload"]["request"].get("runtime").is_none());
        assert!(payload["payload"]["request"].get("respondTo").is_none());
    }

    #[test]
    fn update_requires_a_change() {
        let error = build_update(
            &Keys::generate(),
            &Keys::generate().public_key(),
            UpdateAgentDraft {
                channel_id: CHANNEL.into(),
                agent_name: "Scout".into(),
                display_name: None,
                system_prompt: None,
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("at least one field"));
    }

    #[test]
    fn create_rejects_invalid_channel() {
        let error = build_create(
            &Keys::generate(),
            &Keys::generate().public_key(),
            CreateAgentDraft {
                channel_id: "general".into(),
                display_name: "Scout".into(),
                system_prompt: "Help".into(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid channel UUID"));
    }

    #[test]
    fn project_channel_request_is_owner_encrypted() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let built = build_project_channel(
            &agent,
            &owner.public_key(),
            CreateProjectChannelDraft {
                home_channel_id: CHANNEL.into(),
                name: "release-planning".into(),
                description: Some("Coordinate the next release.".into()),
                visibility: "open".into(),
                ttl_seconds: None,
                template_name: Some("Release team".into()),
            },
        )
        .unwrap();

        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        assert_eq!(payload["kind"], PROJECT_CHANNEL_REQUEST_KIND);
        assert_eq!(payload["channelId"], CHANNEL);
        assert_eq!(payload["payload"]["type"], PROJECT_CHANNEL_REQUEST_KIND);
        assert_eq!(payload["payload"]["action"], "create");
        assert_eq!(payload["payload"]["request"]["homeChannelId"], CHANNEL);
        assert_eq!(
            payload["payload"]["request"]["templateName"],
            "Release team"
        );
    }

    fn folder_draft(folder: &str) -> FolderProjectDraft {
        FolderProjectDraft {
            channel_id: CHANNEL.into(),
            name: "demo-app".into(),
            folder: folder.into(),
            description: None,
            visibility: "open".into(),
        }
    }

    /// Only the owner can read the request, and it carries exactly what the
    /// desktop's parser accepts.
    #[test]
    fn folder_project_request_is_owner_encrypted() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let built = build_folder_project(
            &agent,
            &owner.public_key(),
            folder_draft("/Users/me/Desktop/Demo App - v2"),
        )
        .unwrap();

        assert!(decrypt_observer_payload::<serde_json::Value>(&agent, &built.event).is_err());
        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        assert_eq!(payload["kind"], FOLDER_PROJECT_REQUEST_KIND);
        assert_eq!(payload["channelId"], CHANNEL);
        assert_eq!(payload["payload"]["type"], FOLDER_PROJECT_REQUEST_KIND);
        assert_eq!(payload["payload"]["action"], "create");
        let request = &payload["payload"]["request"];
        assert_eq!(request["channelId"], CHANNEL);
        assert_eq!(request["name"], "demo-app");
        assert_eq!(
            request["folder"],
            "/Users/me/Desktop/Demo App - v2"
        );
        assert_eq!(request["visibility"], "open");
        assert!(request.get("description").is_none());
    }

    #[test]
    fn a_folder_project_names_an_absolute_folder_and_a_real_channel() {
        let keys = Keys::generate();
        let owner = Keys::generate().public_key();
        let relative = build_folder_project(&keys, &owner, folder_draft("Desktop/x")).unwrap_err();
        assert!(relative.to_string().contains("absolute"), "{relative}");

        let mut draft = folder_draft("/tmp/x");
        draft.channel_id = "general".into();
        let channel = build_folder_project(&keys, &owner, draft).unwrap_err();
        assert!(
            channel.to_string().contains("invalid channel UUID"),
            "{channel}"
        );

        let mut draft = folder_draft("/tmp/x");
        draft.visibility = "secret".into();
        assert!(build_folder_project(&keys, &owner, draft).is_err());
    }

    #[test]
    fn a_project_folder_resolves_to_the_canonical_directory() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Demo App - v2");
        std::fs::create_dir(&project).unwrap();
        let canonical = project.canonicalize().unwrap();

        let resolved = resolve_project_folder(&format!("  {}/  ", project.display())).unwrap();
        assert_eq!(resolved, canonical.to_str().unwrap());

        let missing =
            resolve_project_folder(&dir.path().join("nope").display().to_string()).unwrap_err();
        assert!(
            missing.to_string().contains("folder not found"),
            "{missing}"
        );

        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        let not_dir = resolve_project_folder(&file.display().to_string()).unwrap_err();
        assert!(not_dir.to_string().contains("not a folder"), "{not_dir}");

        assert!(resolve_project_folder("relative/path").is_err());
    }
}
