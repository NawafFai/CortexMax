//! What a channel is supposed to produce, and whether it has.
//!
//! A channel's project has an actual goal — a report, a workbook, a build
//! output — but nothing in the app knew what that goal was. Progress could
//! only be judged by reading the team's chat and then going to look on disk.
//! During the live run that is exactly what happened: every input for the
//! workbook existed as 22 CSV files, and nobody noticed for hours that the one
//! script which combines them had never been written.
//!
//! So a channel can declare its targets, and this reports which of them exist,
//! how big they are and when they last changed. It is a filesystem check and
//! says nothing about whether the contents are any good — a zero-byte file is
//! reported as present-but-empty rather than as a finished deliverable,
//! because "the file is there" and "the work is done" are different claims.
//!
//! Targets are always relative to the channel's own bound directory, and a
//! target that tries to leave it is refused rather than clamped. A channel
//! declaring `../../../etc/passwd` must not turn this into a file prober, and
//! a typo must not silently start reporting on a directory the owner never
//! pointed this channel at.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// One declared output and what is actually on disk for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deliverable {
    /// Path as the owner declared it, relative to the channel's directory.
    pub path: String,
    /// Whether a file exists there now.
    pub present: bool,
    /// Size in bytes when present.
    pub size_bytes: Option<u64>,
    /// Last modification, unix seconds, when present.
    pub modified_at: Option<i64>,
    /// Why this target could not be checked at all.
    pub error: Option<String>,
}

/// A channel's declared outputs and how far along they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliverableReport {
    pub targets: Vec<Deliverable>,
    /// Targets that exist and are not empty.
    pub produced: usize,
    /// Targets that exist but contain nothing yet.
    pub empty: usize,
    pub total: usize,
    /// True when the channel has no bound directory to look in.
    pub no_workspace: bool,
}

/// Resolve a declared target inside `root`, refusing anything that leaves it.
///
/// Rejects absolute paths, parent traversal and Windows prefixes. The check is
/// lexical and deliberately runs before touching the filesystem, so a target
/// pointing outside is never even opened.
pub fn resolve_target(root: &Path, declared: &str) -> Result<PathBuf, String> {
    let declared = declared.trim();
    if declared.is_empty() {
        return Err("empty target".to_string());
    }
    let relative = Path::new(declared);

    for component in relative.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("{declared} points outside the channel's folder"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "{declared} must be relative to the channel's folder"
                ));
            }
        }
    }
    Ok(root.join(relative))
}

/// Check one target against the filesystem.
fn check(root: &Path, declared: &str) -> Deliverable {
    let mut out = Deliverable {
        path: declared.to_string(),
        present: false,
        size_bytes: None,
        modified_at: None,
        error: None,
    };

    let resolved = match resolve_target(root, declared) {
        Ok(path) => path,
        Err(error) => {
            out.error = Some(error);
            return out;
        }
    };

    match std::fs::metadata(&resolved) {
        // A directory where a file was declared is a real mismatch worth
        // saying, not a quiet "not produced yet".
        Ok(meta) if meta.is_dir() => {
            out.error = Some("a folder exists at this path, not a file".to_string());
        }
        Ok(meta) => {
            out.present = true;
            out.size_bytes = Some(meta.len());
            out.modified_at = meta
                .modified()
                .ok()
                .and_then(|when| when.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|elapsed| elapsed.as_secs() as i64);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => out.error = Some(error.to_string()),
    }
    out
}

/// Check every declared target for a channel.
pub fn report(root: Option<&Path>, declared: &[String]) -> DeliverableReport {
    let Some(root) = root else {
        return DeliverableReport {
            total: declared.len(),
            no_workspace: true,
            targets: declared
                .iter()
                .map(|path| Deliverable {
                    path: path.clone(),
                    present: false,
                    size_bytes: None,
                    modified_at: None,
                    error: Some("this channel has no folder bound yet".to_string()),
                })
                .collect(),
            ..DeliverableReport::default()
        };
    };

    let targets: Vec<Deliverable> = declared.iter().map(|path| check(root, path)).collect();
    // An empty file is present but is not a produced deliverable. Counting it
    // as done is how a progress bar comes to read 100% over nothing.
    let produced = targets
        .iter()
        .filter(|target| target.present && target.size_bytes.is_some_and(|size| size > 0))
        .count();
    let empty = targets
        .iter()
        .filter(|target| target.present && target.size_bytes == Some(0))
        .count();

    DeliverableReport {
        total: targets.len(),
        produced,
        empty,
        targets,
        no_workspace: false,
    }
}

// ---------------------------------------------------------------------------
// Persistence and commands
// ---------------------------------------------------------------------------

use std::collections::BTreeMap;
use tauri::AppHandle;

use crate::managed_agents::storage::managed_agents_base_dir;

const DELIVERABLES_FILE: &str = "channel-deliverables.json";

/// Declared targets per channel id.
type Declarations = BTreeMap<String, Vec<String>>;

fn declarations_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join(DELIVERABLES_FILE))
}

fn read_declarations(app: &AppHandle) -> Result<Declarations, String> {
    match std::fs::read(declarations_path(app)?) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Declarations::new()),
        Err(error) => Err(error.to_string()),
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
    }
}

fn write_declarations(app: &AppHandle, declarations: &Declarations) -> Result<(), String> {
    let path = declarations_path(app)?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(declarations).map_err(|error| error.to_string())?;
    std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

/// This channel's declared targets and their current state on disk.
///
/// The workspace path is resolved here rather than accepted as an argument.
/// The binding is device-local, and the app's own workspace read is careful
/// never to publish the path to the interface; a command that took one would
/// hand it back out and let any caller point this checker at any directory.
#[tauri::command]
pub async fn channel_deliverables(
    channel_id: String,
    state: tauri::State<'_, crate::app_state::AppState>,
    app: AppHandle,
) -> Result<DeliverableReport, String> {
    let identity = crate::archive::identity_pubkey(&state)?;
    let relay = crate::relay::relay_ws_url_with_override(&state);
    let wanted = channel_id.clone();

    let root = state
        .archive_db
        .with_conn(move |conn| {
            let bindings = crate::archive::channel_workspaces::list(conn, &identity, &relay)?;
            Ok(bindings
                .into_iter()
                .find(|binding| binding.channel_id.to_string() == wanted)
                .and_then(|binding| binding.root))
        })
        .await?;

    tokio::task::spawn_blocking(move || {
        let declared = read_declarations(&app)?
            .get(&channel_id)
            .cloned()
            .unwrap_or_default();
        Ok(report(root.as_deref().map(Path::new), &declared))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Replace this channel's declared targets.
#[tauri::command]
pub async fn set_channel_deliverables(
    channel_id: String,
    targets: Vec<String>,
    app: AppHandle,
) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        let cleaned: Vec<String> = targets
            .into_iter()
            .map(|target| target.trim().to_string())
            .filter(|target| !target.is_empty())
            .collect();

        let mut declarations = read_declarations(&app)?;
        if cleaned.is_empty() {
            declarations.remove(&channel_id);
        } else {
            declarations.insert(channel_id, cleaned.clone());
        }
        write_declarations(&app, &declarations)?;
        Ok(cleaned)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
#[path = "channel_deliverables_tests.rs"]
mod tests;

// ---------------------------------------------------------------------------
// Handover
// ---------------------------------------------------------------------------

/// What happened when a handover note was asked for.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoverResult {
    /// True when this call wrote the file; false when one was already there.
    pub created: bool,
    /// Why nothing could be done, when that is the answer.
    pub error: Option<String>,
}

/// Create a handover note in the channel's folder, but never overwrite one.
///
/// Agents coordinate well inside a conversation and badly across turns: a
/// different agent's session never saw this one's history, and a restarted
/// agent has forgotten its own. A file in the project is the one thing they
/// all read — which this project demonstrated, where the useful coordination
/// already lived in a `HANDOVER.md` the team wrote for itself.
///
/// It refuses to overwrite for that same reason. An existing handover is the
/// team's accumulated account of what is in flight and what must not be built
/// upon; replacing it with a blank template would destroy exactly the content
/// that makes it worth having. An existing file is reported as not created,
/// which is a result rather than a failure.
#[tauri::command]
pub async fn ensure_channel_handover(
    channel_id: String,
    contents: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<HandoverResult, String> {
    let identity = crate::archive::identity_pubkey(&state)?;
    let relay = crate::relay::relay_ws_url_with_override(&state);
    let wanted = channel_id.clone();

    let root = state
        .archive_db
        .with_conn(move |conn| {
            let bindings = crate::archive::channel_workspaces::list(conn, &identity, &relay)?;
            Ok(bindings
                .into_iter()
                .find(|binding| binding.channel_id.to_string() == wanted)
                .and_then(|binding| binding.root))
        })
        .await?;

    let Some(root) = root else {
        return Ok(HandoverResult {
            created: false,
            error: Some("this channel has no folder bound to it".to_string()),
        });
    };
    let root = std::path::PathBuf::from(root);

    tokio::task::spawn_blocking(move || {
        // Through the same resolver the deliverables use, so a handover can
        // never be written outside the folder the channel was pointed at.
        let path = match resolve_target(&root, "HANDOVER.md") {
            Ok(path) => path,
            Err(error) => {
                return Ok(HandoverResult {
                    created: false,
                    error: Some(error),
                })
            }
        };
        if path.exists() {
            return Ok(HandoverResult {
                created: false,
                error: None,
            });
        }
        match std::fs::write(&path, contents) {
            Ok(()) => Ok(HandoverResult {
                created: true,
                error: None,
            }),
            Err(error) => Ok(HandoverResult {
                created: false,
                error: Some(error.to_string()),
            }),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}
