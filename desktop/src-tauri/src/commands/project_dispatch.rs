//! Task board dispatcher.
//!
//! Every task transition that must wake an agent records a *dispatch intent* on
//! the task (`dispatch_state = pending`). This module turns intents into relay
//! messages through the desktop's ordinary message path — a kind 9 message from
//! the owner that mentions the target agent — and confirms the receipt on the
//! task. The message carries a client marker `buzz-project-task:<id>:<revision>`,
//! so an attempt whose outcome is unknown (crash or transport error after the
//! publish) is reconciled against the relay before anything is re-sent. Retries
//! are bounded and never claim exactly-once delivery: after the bound, the intent
//! stays `uncertain` for the owner to resolve.
//!
//! Notifications to `owner` never leave the device: the board itself is the
//! owner's notification.

use crate::{app_state::AppState, archive::project_tasks, events};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

/// Tick interval for the background loop.
const TICK: Duration = Duration::from_secs(5);
/// Maximum intents handled per tick.
const PER_TICK: usize = 20;
/// Maximum publish attempts per task revision before the intent is left to the owner.
const MAX_ATTEMPTS: i64 = 3;
/// Minimum seconds between attempts, multiplied by the attempt number.
const BACKOFF_SECS: i64 = 30;

/// What one dispatch pass did.
#[derive(Debug, Default, Serialize, Clone)]
pub struct DispatchSummary {
    /// Intents published and confirmed this pass.
    pub published: usize,
    /// Owner-targeted intents completed in-app.
    pub in_app: usize,
    /// Uncertain intents confirmed from a relay marker.
    pub reconciled: usize,
    /// Intents that became or stayed uncertain.
    pub uncertain: usize,
    /// Intents skipped for backoff.
    pub deferred: usize,
}

/// Client marker attached to every dispatch message.
pub(crate) fn marker(task_id: &str, revision: i64) -> String {
    format!("buzz-project-task:{task_id}:{revision}")
}

/// Owner-facing message body: Arabic by default, English terms on their own lines.
pub(crate) fn message_body(task: &project_tasks::Task) -> String {
    let headline = match task.state.as_str() {
        "assigned" => {
            "أُسندت إليك مهمة في مشروع هذه القناة. ابدأها بأداة المهام ثم سلّمها بمراجع مخرجاتها."
        }
        "delivered" => "سُلّمت مهمة وتنتظر مراجعة مستقلة أو قرارك.",
        "blocked" => "توقفت مهمة بسبب عائق يحتاج إلى قرار.",
        "waiting_input" => "مهمة تنتظر مدخلات قبل المتابعة.",
        "awaiting_owner_approval" => "اكتملت المراجعة المستقلة؛ المهمة تنتظر اعتماد المالك.",
        _ => "تغيّرت حالة مهمة في لوحة مهام المشروع.",
    };
    let mut lines = vec![
        "[لوحة مهام المشروع]".to_string(),
        headline.to_string(),
        "Task".to_string(),
        task.title.clone(),
        "Task ID".to_string(),
        task.task_id.clone(),
        "State".to_string(),
        task.state.clone(),
    ];
    if let Some(blocker) = &task.blocker {
        lines.push("العائق".into());
        lines.push(blocker.clone());
    }
    if !task.required_inputs.is_empty() {
        lines.push("المدخلات المطلوبة".into());
        lines.extend(task.required_inputs.iter().cloned());
    }
    if !task.outputs.is_empty() {
        lines.push("المخرجات".into());
        lines.extend(
            task.outputs
                .iter()
                .map(|output| format!("{}: {}", output.kind, output.reference)),
        );
    }
    lines.push(
        "استخدم أدوات المهام الأصلية لقراءة التفاصيل وتقديم الحالة التالية؛ لا يُعدّ النثر تسليمًا."
            .into(),
    );
    lines.join("\n")
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One bounded dispatch pass for the active identity and community.
pub(crate) async fn dispatch_once(app: &AppHandle) -> Result<DispatchSummary, String> {
    let state = app.state::<AppState>();
    let Ok(signing_keys) = state.signing_keys() else {
        return Ok(DispatchSummary::default());
    };
    let owner = signing_keys.public_key().to_hex();
    let relay = buzz_core_pkg::relay::normalize_relay_url(
        &crate::relay::relay_ws_url_with_override(&state),
    )
    .map_err(|error| error.to_string())?;
    let relay_base = crate::relay::relay_api_base_url_with_override(&state);
    let backlog = {
        let owner = owner.clone();
        let relay = relay.clone();
        state
            .archive_db
            .with_conn(move |conn| project_tasks::dispatch_backlog(conn, &owner, &relay))
            .await?
    };
    let mut summary = DispatchSummary::default();
    for task in backlog.into_iter().take(PER_TICK) {
        let target = task
            .dispatch_target
            .clone()
            .unwrap_or_else(|| "owner".into());
        let task_id = task.task_id.clone();
        let revision = task.revision;
        let mark = marker(&task_id, revision);

        if target == "owner" {
            let (owner_c, relay_c, id_c) = (owner.clone(), relay.clone(), task_id.clone());
            let outcome = state
                .archive_db
                .with_conn(move |conn| {
                    if task.dispatch_state == "uncertain" {
                        project_tasks::resolve_uncertain_dispatch(
                            conn,
                            &owner_c,
                            &relay_c,
                            &id_c,
                            revision,
                            Some("in-app"),
                        )
                    } else {
                        project_tasks::confirm_dispatch(
                            conn, &owner_c, &relay_c, &id_c, revision, "in-app",
                        )
                    }
                })
                .await;
            if outcome.is_ok() {
                summary.in_app += 1;
            }
            continue;
        }

        // Reconcile first: a pending intent with an attempt already started, or an
        // uncertain one, may already exist on the relay.
        let needs_reconcile =
            task.dispatch_state == "uncertain" || task.dispatch_attempted_at.is_some();
        let dispatch_state = task.dispatch_state.clone();
        if needs_reconcile {
            let observed = crate::commands::messages::find_managed_agent_channel_message_by_marker(
                &state,
                Some(&owner),
                &task.channel_id.to_string(),
                &mark,
            )
            .await;
            match observed {
                Ok(Some(event)) => {
                    let event_id = event.id.to_hex();
                    let (owner_c, relay_c, id_c) = (owner.clone(), relay.clone(), task_id.clone());
                    let was_uncertain = task.dispatch_state == "uncertain";
                    state
                        .archive_db
                        .with_conn(move |conn| {
                            if was_uncertain {
                                project_tasks::resolve_uncertain_dispatch(
                                    conn,
                                    &owner_c,
                                    &relay_c,
                                    &id_c,
                                    revision,
                                    Some(&event_id),
                                )
                            } else {
                                project_tasks::confirm_dispatch(
                                    conn, &owner_c, &relay_c, &id_c, revision, &event_id,
                                )
                            }
                        })
                        .await?;
                    summary.reconciled += 1;
                    continue;
                }
                Ok(None) => {
                    // Not on the relay. Decide whether one more attempt is allowed.
                    let (owner_c, relay_c, id_c) = (owner.clone(), relay.clone(), task_id.clone());
                    let attempts = state
                        .archive_db
                        .with_conn(move |conn| {
                            let attempts = project_tasks::dispatch_attempts(conn, &id_c, revision)?;
                            if attempts < MAX_ATTEMPTS {
                                if dispatch_state == "uncertain" {
                                    project_tasks::resolve_uncertain_dispatch(
                                        conn, &owner_c, &relay_c, &id_c, revision, None,
                                    )?;
                                } else {
                                    // Pending with a stale attempt marker: treat as uncertain, then reopen.
                                    project_tasks::mark_dispatch_uncertain(
                                        conn, &owner_c, &relay_c, &id_c, revision,
                                    )?;
                                    project_tasks::resolve_uncertain_dispatch(
                                        conn, &owner_c, &relay_c, &id_c, revision, None,
                                    )?;
                                }
                            } else if dispatch_state != "uncertain" {
                                project_tasks::mark_dispatch_uncertain(
                                    conn, &owner_c, &relay_c, &id_c, revision,
                                )?;
                            }
                            Ok(attempts)
                        })
                        .await?;
                    if attempts >= MAX_ATTEMPTS {
                        summary.uncertain += 1;
                        continue;
                    }
                    // Backoff between attempts.
                    if let Some(last) = task.dispatch_attempted_at {
                        if now() - last < BACKOFF_SECS * attempts.max(1) {
                            summary.deferred += 1;
                            continue;
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!("task dispatch reconciliation for {task_id} failed: {error}");
                    summary.uncertain += 1;
                    continue;
                }
            }
        }

        // Fresh attempt: journal it before the side effect.
        let (owner_c, relay_c, id_c) = (owner.clone(), relay.clone(), task_id.clone());
        let begun = state
            .archive_db
            .with_conn(move |conn| {
                project_tasks::begin_dispatch(conn, &owner_c, &relay_c, &id_c, revision)?;
                project_tasks::note_dispatch(
                    conn,
                    &id_c,
                    revision,
                    "dispatch_attempt",
                    "relay message",
                )
            })
            .await;
        if begun.is_err() {
            summary.deferred += 1;
            continue;
        }
        let builder = events::build_message_with_client_tags(
            task.channel_id,
            &message_body(&task),
            None,
            &[target.as_str()],
            &[],
            &[],
            &[],
            &[],
            None,
            &relay_base,
            &[vec!["client".to_string(), mark.clone()]],
        )?;
        let published =
            crate::relay::submit_event_at_created_at(builder, &state, &relay_base, &signing_keys)
                .await;
        let (owner_c, relay_c, id_c) = (owner.clone(), relay.clone(), task_id.clone());
        match published {
            Ok((response, _)) => {
                let event_id = response.event_id.clone();
                state
                    .archive_db
                    .with_conn(move |conn| {
                        project_tasks::confirm_dispatch(
                            conn, &owner_c, &relay_c, &id_c, revision, &event_id,
                        )
                    })
                    .await?;
                summary.published += 1;
            }
            Err(error) => {
                tracing::warn!("task dispatch publish for {task_id} failed: {error}");
                state
                    .archive_db
                    .with_conn(move |conn| {
                        project_tasks::mark_dispatch_uncertain(
                            conn, &owner_c, &relay_c, &id_c, revision,
                        )
                    })
                    .await?;
                summary.uncertain += 1;
            }
        }
    }
    Ok(summary)
}

/// Run the dispatcher loop for the app's lifetime. Errors are logged; a failing
/// tick never stops the loop and never retries a side effect on its own.
pub(crate) fn spawn_dispatch_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            if let Err(error) = dispatch_once(&app).await {
                tracing::debug!("task dispatch tick skipped: {error}");
            }
        }
    });
}

/// Run one dispatch pass now (used by the UI after an owner action and by tests).
#[tauri::command]
pub async fn dispatch_channel_project_tasks(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_relay: String,
    expected_owner: String,
) -> Result<DispatchSummary, String> {
    super::channel_workspaces::scope(&state, &expected_relay, &expected_owner)?;
    dispatch_once(&app).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_message_keeps_english_terms_on_their_own_lines() {
        let task = project_tasks::Task {
            task_id: "t-1".into(),
            channel_id: uuid::Uuid::nil(),
            title: "Fix login".into(),
            description: "d".into(),
            state: "assigned".into(),
            state_before_pause: None,
            assignee_pubkey: Some("agent".into()),
            created_by: "owner".into(),
            required_inputs: vec![],
            dependencies: vec![],
            outputs: vec![],
            return_to: "owner".into(),
            blocker: None,
            review: None,
            report: None,
            revision: 2,
            binding_generation: 1,
            team_generation: 1,
            created_at: 0,
            updated_at: 0,
            dispatch_state: "pending".into(),
            dispatch_target: Some("agent".into()),
            dispatch_event_id: None,
            dispatch_attempted_at: None,
        };
        let body = message_body(&task);
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[0], "[لوحة مهام المشروع]");
        assert!(lines.contains(&"Task ID"));
        assert!(lines.contains(&"t-1"));
        assert!(lines.contains(&"State"));
        assert!(lines.contains(&"assigned"));
        assert_eq!(marker("t-1", 2), "buzz-project-task:t-1:2");
    }
}
