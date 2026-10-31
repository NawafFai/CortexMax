//! Reading and setting the channels the owner can simply talk in.
//!
//! See [`crate::archive::channel_conversations`] for what the setting means
//! and why it is scoped to the owner's own messages.

use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::archive::channel_conversations;

/// Read the set at spawn time, off the async runtime.
///
/// Mirrors `channel_workspaces::launch_snapshot`: agent launch is synchronous
/// and may already be inside a runtime, so the read happens on its own thread.
///
/// A failure here returns the empty set rather than an error. Conversation
/// mode is a convenience; refusing to start an agent because its database
/// could not be read would trade a working team for a nicety.
pub(crate) fn launch_conversation_channels(app: &AppHandle, owner: &str, relay: &str) -> Vec<Uuid> {
    let owner = owner.to_string();
    let relay = relay.to_string();
    let read = std::thread::scope(|threads| {
        threads
            .spawn(move || {
                let state = app.state::<AppState>();
                tauri::async_runtime::block_on(
                    state
                        .archive_db
                        .with_conn(move |conn| channel_conversations::list(conn, &owner, &relay)),
                )
            })
            .join()
            .map_err(|_| "conversation channel reader failed".to_string())
    });
    match read {
        Ok(Ok(channels)) => channels,
        Ok(Err(error)) | Err(error) => {
            tracing::warn!("conversation channels unreadable, launching mention-only: {error}");
            Vec::new()
        }
    }
}

/// The channels the owner can talk in without tagging anyone.
#[tauri::command]
pub async fn list_conversation_channels(
    state: State<'_, AppState>,
    relay_url: String,
    owner_pubkey: String,
) -> Result<Vec<Uuid>, String> {
    let relay = buzz_core_pkg::relay::normalize_relay_url(&relay_url).map_err(|e| e.to_string())?;
    state
        .archive_db
        .with_conn(move |conn| channel_conversations::list(conn, &owner_pubkey, &relay))
        .await
}

/// Turn conversation mode on or off for one channel.
///
/// Returns the full set afterwards. The change reaches an agent when it next
/// starts, because a subscription is fixed at launch — the caller is expected
/// to say so rather than let the owner wonder why nothing changed.
#[tauri::command]
pub async fn set_channel_conversation_mode(
    state: State<'_, AppState>,
    relay_url: String,
    owner_pubkey: String,
    channel_id: Uuid,
    enabled: bool,
) -> Result<Vec<Uuid>, String> {
    let relay = buzz_core_pkg::relay::normalize_relay_url(&relay_url).map_err(|e| e.to_string())?;
    state
        .archive_db
        .with_conn(move |conn| {
            channel_conversations::set(conn, &owner_pubkey, &relay, channel_id, enabled)
        })
        .await
}
