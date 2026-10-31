//! Handoff inspection and change review for channel projects.
//!
//! Everything here is read-only against the project: the sandboxed checker reads
//! Git state and diffs, the handoff file is read with a size bound, and the only
//! writes are to the device-local archive (handoff records and the owner's
//! acknowledgement). Nothing commits, merges, pushes, resets, or deploys.

use super::channel_workspaces::scope;
use crate::{
    app_state::AppState,
    archive::{channel_workspaces, project_handoffs::HandoffRecord, project_tasks},
    project_check_runtime::{self, ProjectChanges},
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;
use uuid::Uuid;

/// Handoff file names looked up, in order, relative to the project root.
const HANDOFF_CANDIDATES: &[&str] = &[
    "HANDOFF.md",
    "handoff.md",
    "docs/HANDOFF.md",
    "docs/handoff.md",
    ".buzz/HANDOFF.md",
    "CHECKPOINT.md",
    "docs/CHECKPOINT.md",
    "docs/channel-workspaces-checkpoint.md",
];

/// A process, other than this desktop and its managed agents, whose working
/// directory is inside the project. Evidence of a possible concurrent writer.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ForeignProcess {
    /// Process id.
    pub pid: u32,
    /// Command name as reported by the system.
    pub command: String,
    /// Working directory.
    pub cwd: String,
}

/// What the handoff inspection established.
#[derive(Serialize)]
pub struct HandoffInspection {
    /// Channel inspected.
    pub channel_id: Uuid,
    /// Binding generation held for the inspection.
    pub binding_generation: i64,
    /// Unix seconds.
    pub checked_at: i64,
    /// The stored record (file text, or the previously imported summary if no file exists).
    pub record: Option<HandoffRecord>,
    /// Candidate paths that were probed, for transparency.
    pub probed: Vec<String>,
    /// Markdown checklist items still open in the handoff text.
    pub unfinished_items: Vec<String>,
    /// Paths mentioned in the handoff text that exist in the project.
    pub referenced_paths: Vec<String>,
    /// Live Git state now (not the checkpoint's state).
    pub live: LiveState,
    /// Processes working inside the project that Buzz does not manage.
    pub foreign_processes: Vec<ForeignProcess>,
    /// Whether the foreign-process scan ran; `false` means unknown, not clear.
    pub foreign_scan_complete: bool,
}

/// Current Git facts from the sandboxed checker.
#[derive(Serialize, Default)]
pub struct LiveState {
    /// Branch now.
    pub branch: Option<String>,
    /// Revision now.
    pub revision: Option<String>,
    /// Dirty now.
    pub dirty: Option<bool>,
    /// Changed paths now (bounded).
    pub changed_paths: Vec<project_check_runtime::ChangedPath>,
}

/// Extract `- [ ]` items (bounded).
pub(crate) fn unfinished_items(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- [ ]") || line.starts_with("* [ ]"))
        .map(|line| line[5..].trim().to_string())
        .filter(|item| !item.is_empty())
        .take(64)
        .collect()
}

/// Tokens that look like relative paths and exist under `root` (bounded).
pub(crate) fn referenced_paths(root: &Path, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for token in text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '`' | '(' | ')' | '[' | ']' | '"' | '\'' | ',' | ';' | ':'
            )
    }) {
        if found.len() >= 64 {
            break;
        }
        if token.is_empty()
            || token.len() > 256
            || !token.contains('/') && !token.contains('.')
            || token.starts_with('/')
            || token.contains("..")
            || token.starts_with("http")
        {
            continue;
        }
        let candidate = root.join(token);
        if candidate.is_file() && !found.contains(&token.to_string()) {
            found.push(token.to_string());
        }
    }
    found
}

/// Processes whose working directory is under `root`, excluding this desktop
/// and pids it manages. Uses `lsof -d cwd`, bounded by a 5 s deadline.
async fn foreign_processes(root: &Path, managed: &[u32]) -> Option<Vec<ForeignProcess>> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new("/usr/sbin/lsof")
            .args(["-F", "pcn", "-d", "cwd", "-n"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let own = std::process::id();
    let mut processes = Vec::new();
    let (mut pid, mut command) = (None::<u32>, String::new());
    for line in text.lines() {
        match line.chars().next() {
            Some('p') => {
                pid = line[1..].parse().ok();
                command.clear();
            }
            Some('c') => command = line[1..].to_string(),
            Some('n') => {
                let cwd = Path::new(&line[1..]);
                if let Some(pid) = pid {
                    if pid != own
                        && !managed.contains(&pid)
                        && cwd.starts_with(root)
                        && processes.len() < 64
                    {
                        processes.push(ForeignProcess {
                            pid,
                            command: command.clone(),
                            cwd: cwd.to_string_lossy().into_owned(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    Some(processes)
}

fn now() -> Result<i64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .map_err(|error| error.to_string())
}

fn bound_root(
    conn: &rusqlite::Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    expected_binding_generation: i64,
) -> Result<PathBuf, String> {
    let binding = channel_workspaces::list(conn, owner, relay)?
        .into_iter()
        .find(|binding| binding.channel_id == channel)
        .ok_or("attach a project first")?;
    if binding.generation != expected_binding_generation {
        return Err("project binding changed; reload before inspecting".into());
    }
    channel_workspaces::validate_root(binding.root.as_deref().ok_or("attach a project first")?)
}

/// Inspect the project's handoff file (if any), live Git state, and possible
/// concurrent writers. Stores the file text as the channel's handoff record.
#[tauri::command]
pub async fn inspect_channel_project_handoff(
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
    expected_binding_generation: i64,
) -> Result<HandoffInspection, String> {
    let _changes = std::sync::Arc::clone(&state.channel_workspace_change_lock)
        .read_owned()
        .await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let (owner_c, relay_c) = (owner.clone(), relay.clone());
    let root = state
        .archive_db
        .with_conn(move |conn| {
            bound_root(
                conn,
                &owner_c,
                &relay_c,
                channel_id,
                expected_binding_generation,
            )
        })
        .await?;
    let checked_at = now()?;
    let mut probed = Vec::new();
    let mut file: Option<(String, String)> = None;
    for candidate in HANDOFF_CANDIDATES {
        probed.push((*candidate).to_string());
        let path = root.join(candidate);
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        if metadata.len() as usize > crate::archive::project_handoffs::MAX_HANDOFF_BYTES {
            return Err(format!(
                "handoff file {candidate} exceeds the {} byte bound; import a summary instead",
                crate::archive::project_handoffs::MAX_HANDOFF_BYTES
            ));
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("handoff file {candidate} unreadable: {error}"))?;
        file = Some(((*candidate).to_string(), text));
        break;
    }
    let evidence = project_check_runtime::inspect(root.clone()).await?;
    let changes = project_check_runtime::changes(root.clone()).await?;
    let managed: Vec<u32> = state
        .managed_agent_processes
        .lock()
        .map(|processes| {
            processes
                .values()
                .map(|pair| pair.process.child.id())
                .collect()
        })
        .unwrap_or_default();
    let foreign = foreign_processes(&root, &managed).await;
    let (owner_c, relay_c) = (owner.clone(), relay.clone());
    let live_revision = evidence.revision.clone();
    let live_dirty = evidence.dirty;
    let record = state
        .archive_db
        .with_conn(move |conn| {
            let existing =
                crate::archive::project_handoffs::load(conn, &owner_c, &relay_c, channel_id)?;
            match file {
                Some((label, text)) => {
                    // Re-capturing an unchanged file keeps the acknowledgement; new text resets it.
                    let previous_writer_released_at = existing
                        .as_ref()
                        .filter(|record| {
                            record.source == "file" && record.label == label && record.text == text
                        })
                        .and_then(|record| record.previous_writer_released_at);
                    let record = HandoffRecord {
                        source: "file".into(),
                        label,
                        text,
                        captured_at: checked_at,
                        binding_generation: expected_binding_generation,
                        git_revision: live_revision,
                        dirty_at_capture: live_dirty,
                        previous_writer_released_at,
                    };
                    crate::archive::project_handoffs::store(
                        conn, &owner_c, &relay_c, channel_id, &record,
                    )?;
                    Ok(Some(record))
                }
                None => Ok(existing),
            }
        })
        .await?;
    scope(&state, &expected_relay, &expected_owner)?;
    Ok(HandoffInspection {
        channel_id,
        binding_generation: expected_binding_generation,
        checked_at,
        unfinished_items: record
            .as_ref()
            .map(|r| unfinished_items(&r.text))
            .unwrap_or_default(),
        referenced_paths: record
            .as_ref()
            .map(|r| referenced_paths(&root, &r.text))
            .unwrap_or_default(),
        record,
        probed,
        live: LiveState {
            branch: evidence.branch,
            revision: evidence.revision,
            dirty: evidence.dirty,
            changed_paths: changes.changed_paths,
        },
        foreign_scan_complete: foreign.is_some(),
        foreign_processes: foreign.unwrap_or_default(),
    })
}

/// Store an owner-supplied summary (for example pasted from another provider's
/// session export). Buzz never reads another provider's history itself.
#[tauri::command]
pub async fn import_channel_project_handoff_summary(
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
    expected_binding_generation: i64,
    label: String,
    text: String,
) -> Result<HandoffRecord, String> {
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let label = label.trim().to_string();
    let text = text.trim().to_string();
    if label.is_empty() || label.len() > 200 {
        return Err("give the imported summary a short label".into());
    }
    if text.is_empty() {
        return Err("the imported summary is empty".into());
    }
    if text.len() > crate::archive::project_handoffs::MAX_HANDOFF_BYTES {
        return Err("the imported summary exceeds the stored bound".into());
    }
    let captured_at = now()?;
    state
        .archive_db
        .with_conn(move |conn| {
            bound_root(
                conn,
                &owner,
                &relay,
                channel_id,
                expected_binding_generation,
            )?;
            let record = HandoffRecord {
                source: "imported".into(),
                label,
                text,
                captured_at,
                binding_generation: expected_binding_generation,
                git_revision: None,
                dirty_at_capture: None,
                previous_writer_released_at: None,
            };
            crate::archive::project_handoffs::store(conn, &owner, &relay, channel_id, &record)?;
            Ok(record)
        })
        .await
}

/// Record the owner's explicit confirmation that the previous writer released
/// the project. This is an owner statement, not an automated check.
#[tauri::command]
pub async fn acknowledge_channel_project_previous_writer_release(
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
    confirmed: bool,
) -> Result<HandoffRecord, String> {
    if !confirmed {
        return Err("explicit confirmation is required".into());
    }
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let at = now()?;
    state
        .archive_db
        .with_conn(move |conn| {
            crate::archive::project_handoffs::acknowledge_release(
                conn, &owner, &relay, channel_id, at,
            )
        })
        .await
}

/// Files that more than one active task lists as an output.
#[derive(Serialize)]
pub struct OutputOverlap {
    /// Relative path.
    pub path: String,
    /// Task ids that reference it.
    pub task_ids: Vec<String>,
}

/// Change review for one project: what changed on disk, what tasks claim it,
/// and which native edits are recorded, so acceptance can be an informed act.
#[derive(Serialize)]
pub struct ProjectChangeReview {
    /// Channel reviewed.
    pub channel_id: Uuid,
    /// Binding generation held for the review.
    pub binding_generation: i64,
    /// Unix seconds.
    pub checked_at: i64,
    /// Working-tree changes from the sandboxed checker.
    pub changes: ProjectChanges,
    /// Output paths claimed by more than one active task.
    pub overlaps: Vec<OutputOverlap>,
    /// Native edit receipts recorded for this channel (completed and pending).
    pub edit_receipts: Vec<serde_json::Value>,
    /// Task ids whose delivery is awaiting review or acceptance.
    pub awaiting: Vec<String>,
}

/// Read the working-tree diff and cross-reference it with the task board.
#[tauri::command]
pub async fn get_channel_project_changes(
    state: State<'_, AppState>,
    channel_id: Uuid,
    expected_relay: String,
    expected_owner: String,
    expected_binding_generation: i64,
) -> Result<ProjectChangeReview, String> {
    let _changes = std::sync::Arc::clone(&state.channel_workspace_change_lock)
        .read_owned()
        .await;
    let (owner, relay) = scope(&state, &expected_relay, &expected_owner)?;
    let (owner_c, relay_c) = (owner.clone(), relay.clone());
    let root = state
        .archive_db
        .with_conn(move |conn| {
            bound_root(
                conn,
                &owner_c,
                &relay_c,
                channel_id,
                expected_binding_generation,
            )
        })
        .await?;
    let changes = project_check_runtime::changes(root).await?;
    let (owner_c, relay_c) = (owner.clone(), relay.clone());
    let (overlaps, edit_receipts, awaiting) = state
        .archive_db
        .with_conn(move |conn| {
            let tasks = project_tasks::list(conn, &owner_c, &relay_c, channel_id)?;
            let mut by_path: std::collections::BTreeMap<String, Vec<String>> = Default::default();
            let mut awaiting = Vec::new();
            for task in &tasks {
                if matches!(task.state.as_str(), "completed" | "cancelled") {
                    continue;
                }
                if matches!(task.state.as_str(), "delivered" | "under_review" | "awaiting_owner_approval") {
                    awaiting.push(task.task_id.clone());
                }
                for output in &task.outputs {
                    if output.kind == "file" {
                        by_path.entry(output.reference.clone()).or_default().push(task.task_id.clone());
                    }
                }
            }
            let overlaps = by_path
                .into_iter()
                .filter(|(_, ids)| ids.len() > 1)
                .map(|(path, task_ids)| OutputOverlap { path, task_ids })
                .collect();
            let mut statement = conn
                .prepare(
                    "SELECT operation_id, relative_path, before_sha256, after_sha256, state, created_at
                     FROM project_file_operations
                     WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3
                     ORDER BY created_at DESC LIMIT 200",
                )
                .map_err(|error| error.to_string())?;
            let receipts = statement
                .query_map(rusqlite::params![owner_c, relay_c, channel_id.to_string()], |row| {
                    Ok(serde_json::json!({
                        "operation_id": row.get::<_, String>(0)?,
                        "relative_path": row.get::<_, String>(1)?,
                        "before_sha256": row.get::<_, String>(2)?,
                        "after_sha256": row.get::<_, String>(3)?,
                        "state": row.get::<_, String>(4)?,
                        "created_at": row.get::<_, i64>(5)?,
                    }))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            Ok((overlaps, receipts, awaiting))
        })
        .await?;
    scope(&state, &expected_relay, &expected_owner)?;
    Ok(ProjectChangeReview {
        channel_id,
        binding_generation: expected_binding_generation,
        checked_at: now()?,
        changes,
        overlaps,
        edit_receipts,
        awaiting,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_text_extraction_is_bounded_and_path_aware() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        let text = "# Handoff\n- [x] done thing\n- [ ] finish `src/main.rs` tests\n* [ ] update docs/missing.md\nSee src/main.rs and ../outside.rs";
        assert_eq!(
            unfinished_items(text),
            vec![
                "finish `src/main.rs` tests".to_string(),
                "update docs/missing.md".to_string()
            ]
        );
        assert_eq!(
            referenced_paths(dir.path(), text),
            vec!["src/main.rs".to_string()]
        );
    }
}
