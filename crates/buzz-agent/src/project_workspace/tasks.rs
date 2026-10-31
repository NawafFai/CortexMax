//! Channel project task board: the authoritative, device-local lifecycle record.
//!
//! Lifecycle state advances only through the transitions below, applied by an
//! authenticated actor (the desktop owner, or an agent through the native project
//! tools whose identity the harness established). Model prose never advances a
//! task. Every transition is fenced by the task revision and by the binding and
//! team generations it was created under, so a stale caller cannot overwrite a
//! newer record and a project change never silently redirects work.
//!
//! Notifications to the return destination are recorded as a *dispatch intent*
//! (`dispatch_state = pending`); the desktop publishes them through its existing
//! message path and confirms the receipt. A crash between publish and confirm
//! leaves the intent `uncertain`, which is surfaced to the owner and never
//! retried automatically.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// Every task state, in the order the owner reads them.
pub const TASK_STATES: &[&str] = &[
    "planned",
    "waiting_input",
    "assigned",
    "running",
    "delivered",
    "under_review",
    "blocked",
    "awaiting_owner_approval",
    "completed",
    "paused",
    "cancelled",
];

/// Upper bound on tasks per channel (all states).
pub const MAX_TASKS_PER_CHANNEL: usize = 512;
const MAX_TEXT: usize = 8 * 1024;
const MAX_LIST: usize = 32;

/// One deliverable reference attached to a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputRef {
    /// `file`, `edit_receipt`, `message`, `note`, or `check`.
    pub kind: String,
    /// Path, receipt id, event id, or free text; bounded.
    pub reference: String,
    /// Optional content hash for file outputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// Independent review verdict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRecord {
    /// Reviewer identity (never the assignee).
    pub reviewer_pubkey: String,
    /// `approved` or `changes_requested`.
    pub verdict: String,
    /// Reviewer notes.
    pub notes: String,
    /// Unix seconds.
    pub reviewed_at: i64,
}

/// Coordinator's owner-facing final report, stored verbatim and rendered by the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalReport {
    /// What was requested.
    pub requested: String,
    /// What was actually delivered.
    pub delivered: String,
    /// What was verified, and how.
    pub verified: String,
    /// What failed or remains unverified.
    pub failed_or_unverified: String,
    /// What decision or approval is needed next.
    pub decision_needed: String,
    /// Identity that filed it.
    pub author_pubkey: String,
    /// Unix seconds.
    pub filed_at: i64,
}

/// One task row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Stable identifier.
    pub task_id: String,
    /// Channel whose project the task belongs to.
    pub channel_id: Uuid,
    /// Short title.
    pub title: String,
    /// Bounded task statement.
    pub description: String,
    /// One of [`TASK_STATES`].
    pub state: String,
    /// State before `paused`, restored on resume.
    pub state_before_pause: Option<String>,
    /// Current assignee.
    pub assignee_pubkey: Option<String>,
    /// Identity that created the task (owner or a coordinator with `assign_tasks`).
    pub created_by: String,
    /// Inputs the assignee still needs.
    pub required_inputs: Vec<String>,
    /// Task ids that must be completed first.
    pub dependencies: Vec<String>,
    /// Deliverable references.
    pub outputs: Vec<OutputRef>,
    /// Identity that receives the delivery: an agent pubkey or `owner`.
    pub return_to: String,
    /// Current blocker text, if blocked.
    pub blocker: Option<String>,
    /// Independent review, if filed.
    pub review: Option<ReviewRecord>,
    /// Final report, if filed.
    pub report: Option<FinalReport>,
    /// Monotonic revision; every transition increments it.
    pub revision: i64,
    /// Binding generation the task was created under.
    pub binding_generation: i64,
    /// Team generation the task was created under.
    pub team_generation: i64,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds.
    pub updated_at: i64,
    /// `none`, `pending`, `sent`, or `uncertain`.
    pub dispatch_state: String,
    /// Who the pending/sent dispatch targets.
    pub dispatch_target: Option<String>,
    /// Relay event id of the confirmed dispatch.
    pub dispatch_event_id: Option<String>,
    /// Unix seconds the last dispatch attempt started.
    pub dispatch_attempted_at: Option<i64>,
}

/// Who is acting. The desktop passes `Owner`; native tools pass `Agent` with the
/// identity the harness established, never an identity from tool arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor<'a> {
    /// Device owner.
    Owner,
    /// Existing project team member.
    Agent(&'a str),
}

impl Actor<'_> {
    fn id(&self) -> String {
        match self {
            Actor::Owner => "owner".to_string(),
            Actor::Agent(pubkey) => (*pubkey).to_string(),
        }
    }
}

/// Team facts the state machine needs; the caller loads them from `teams`.
pub struct TeamView<'a> {
    /// Coordinator pubkey, if any.
    pub coordinator: Option<&'a str>,
    /// `(pubkey, capabilities)` for each member.
    pub members: &'a [(String, Vec<String>)],
    /// Whether dispatch is paused.
    pub paused: bool,
    /// Current binding generation.
    pub binding_generation: i64,
    /// Current team generation.
    pub team_generation: i64,
}

impl TeamView<'_> {
    fn has(&self, pubkey: &str, capability: &str) -> bool {
        self.members
            .iter()
            .any(|(member, caps)| member == pubkey && caps.iter().any(|c| c == capability))
    }

    fn is_member(&self, pubkey: &str) -> bool {
        self.members.iter().any(|(member, _)| member == pubkey)
    }
}

/// Create the task tables.
pub fn migrate(conn: &Connection) -> Result<(), String> {
    let is_applied = |connection: &Connection| -> Result<bool, String> {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM archive_migrations WHERE name = 'channel_project_tasks_v1')",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    };
    if is_applied(conn)? {
        return Ok(());
    }
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !is_applied(&transaction)? {
        transaction
            .execute_batch(
                "CREATE TABLE channel_project_tasks (
                    task_id TEXT PRIMARY KEY,
                    identity_pubkey TEXT NOT NULL,
                    relay_url TEXT NOT NULL,
                    channel_id TEXT NOT NULL,
                    title TEXT NOT NULL,
                    description TEXT NOT NULL,
                    state TEXT NOT NULL,
                    state_before_pause TEXT,
                    assignee_pubkey TEXT,
                    created_by TEXT NOT NULL,
                    required_inputs TEXT NOT NULL,
                    dependencies TEXT NOT NULL,
                    outputs TEXT NOT NULL,
                    return_to TEXT NOT NULL,
                    blocker TEXT,
                    review TEXT,
                    report TEXT,
                    revision INTEGER NOT NULL,
                    binding_generation INTEGER NOT NULL,
                    team_generation INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    dispatch_state TEXT NOT NULL DEFAULT 'none',
                    dispatch_target TEXT,
                    dispatch_event_id TEXT,
                    dispatch_attempted_at INTEGER
                );
                CREATE INDEX channel_project_tasks_scope
                    ON channel_project_tasks(identity_pubkey, relay_url, channel_id, state);
                CREATE INDEX channel_project_tasks_dispatch
                    ON channel_project_tasks(identity_pubkey, relay_url, dispatch_state);
                CREATE TABLE channel_project_task_events (
                    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    task_id TEXT NOT NULL,
                    revision INTEGER NOT NULL,
                    actor TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    detail TEXT NOT NULL,
                    at INTEGER NOT NULL
                );
                CREATE INDEX channel_project_task_events_task
                    ON channel_project_task_events(task_id, event_id);
                INSERT INTO archive_migrations(name, applied_at)
                VALUES ('channel_project_tasks_v1', unixepoch());",
            )
            .map_err(|error| format!("project task migration: {error}"))?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

fn now() -> Result<i64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| error.to_string())
}

fn bounded(text: &str, what: &str) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("{what} must not be empty"));
    }
    if trimmed.len() > MAX_TEXT {
        return Err(format!("{what} exceeds {MAX_TEXT} bytes"));
    }
    Ok(trimmed.to_string())
}

fn bounded_list(items: &[String], what: &str) -> Result<Vec<String>, String> {
    if items.len() > MAX_LIST {
        return Err(format!("{what} exceeds {MAX_LIST} entries"));
    }
    items.iter().map(|item| bounded(item, what)).collect()
}

fn json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

fn from_json<T: for<'de> Deserialize<'de>>(raw: &str, what: &str) -> Result<T, String> {
    serde_json::from_str(raw).map_err(|_| format!("invalid stored task {what}"))
}

const SELECT: &str = "SELECT task_id, channel_id, title, description, state, state_before_pause,
    assignee_pubkey, created_by, required_inputs, dependencies, outputs, return_to, blocker, review,
    report, revision, binding_generation, team_generation, created_at, updated_at, dispatch_state,
    dispatch_target, dispatch_event_id, dispatch_attempted_at FROM channel_project_tasks";

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let channel: String = row.get(1)?;
    let convert = |index: usize, error: String| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error)),
        )
    };
    let required_inputs: String = row.get(8)?;
    let dependencies: String = row.get(9)?;
    let outputs: String = row.get(10)?;
    let review: Option<String> = row.get(13)?;
    let report: Option<String> = row.get(14)?;
    Ok(Task {
        task_id: row.get(0)?,
        channel_id: Uuid::parse_str(&channel).map_err(|error| convert(1, error.to_string()))?,
        title: row.get(2)?,
        description: row.get(3)?,
        state: row.get(4)?,
        state_before_pause: row.get(5)?,
        assignee_pubkey: row.get(6)?,
        created_by: row.get(7)?,
        required_inputs: from_json(&required_inputs, "inputs").map_err(|e| convert(8, e))?,
        dependencies: from_json(&dependencies, "dependencies").map_err(|e| convert(9, e))?,
        outputs: from_json(&outputs, "outputs").map_err(|e| convert(10, e))?,
        return_to: row.get(11)?,
        blocker: row.get(12)?,
        review: review
            .map(|raw| from_json(&raw, "review"))
            .transpose()
            .map_err(|e| convert(13, e))?,
        report: report
            .map(|raw| from_json(&raw, "report"))
            .transpose()
            .map_err(|e| convert(14, e))?,
        revision: row.get(15)?,
        binding_generation: row.get(16)?,
        team_generation: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
        dispatch_state: row.get(20)?,
        dispatch_target: row.get(21)?,
        dispatch_event_id: row.get(22)?,
        dispatch_attempted_at: row.get(23)?,
    })
}

/// Load one task in scope.
pub fn get(
    conn: &Connection,
    owner: &str,
    relay: &str,
    task_id: &str,
) -> Result<Option<Task>, String> {
    conn.query_row(
        &format!("{SELECT} WHERE identity_pubkey=?1 AND relay_url=?2 AND task_id=?3"),
        params![owner, relay, task_id],
        row_to_task,
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// List a channel's tasks, newest first. Bounded.
pub fn list(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
) -> Result<Vec<Task>, String> {
    let mut statement = conn
        .prepare(&format!(
            "{SELECT} WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3
             ORDER BY created_at DESC, task_id LIMIT {MAX_TASKS_PER_CHANNEL}"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![owner, relay, channel.to_string()], row_to_task)
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// Tasks whose dispatch intent is pending or uncertain, across all channels.
pub fn dispatch_backlog(conn: &Connection, owner: &str, relay: &str) -> Result<Vec<Task>, String> {
    let mut statement = conn
        .prepare(&format!(
            "{SELECT} WHERE identity_pubkey=?1 AND relay_url=?2
             AND dispatch_state IN ('pending','uncertain') ORDER BY updated_at LIMIT 256"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![owner, relay], row_to_task)
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// Count tasks in `state` for a channel.
pub fn count_in_state(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    state: &str,
) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM channel_project_tasks
         WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3 AND state=?4",
        params![owner, relay, channel.to_string(), state],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

/// Input for [`create`].
pub struct NewTask<'a> {
    /// Short title.
    pub title: &'a str,
    /// Task statement.
    pub description: &'a str,
    /// Inputs the assignee needs before starting.
    pub required_inputs: &'a [String],
    /// Task ids that must complete first.
    pub dependencies: &'a [String],
    /// Delivery destination: an agent pubkey or `owner`.
    pub return_to: &'a str,
}

fn record_event(
    conn: &Connection,
    task_id: &str,
    revision: i64,
    actor: &str,
    kind: &str,
    detail: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO channel_project_task_events(task_id, revision, actor, kind, detail, at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            task_id,
            revision,
            actor,
            kind,
            detail.chars().take(2048).collect::<String>(),
            now()?
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn require_creator(actor: Actor<'_>, team: &TeamView<'_>) -> Result<(), String> {
    match actor {
        Actor::Owner => Ok(()),
        Actor::Agent(pubkey) => {
            if team.has(pubkey, "assign_tasks") {
                Ok(())
            } else {
                Err("only the owner or an agent with assign_tasks may plan or assign tasks".into())
            }
        }
    }
}

/// Create a task in `planned` (or `waiting_input` when inputs or dependencies are outstanding).
pub fn create(
    conn: &Connection,
    owner: &str,
    relay: &str,
    channel: Uuid,
    actor: Actor<'_>,
    team: &TeamView<'_>,
    input: NewTask<'_>,
) -> Result<Task, String> {
    require_creator(actor, team)?;
    let title = bounded(input.title, "title")?;
    let description = bounded(input.description, "description")?;
    let required_inputs = bounded_list(input.required_inputs, "required input")?;
    let dependencies = bounded_list(input.dependencies, "dependency")?;
    let return_to = bounded(input.return_to, "return destination")?;
    if return_to != "owner" && !team.is_member(&return_to) {
        return Err("return destination must be `owner` or a current team member".into());
    }
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM channel_project_tasks WHERE identity_pubkey=?1 AND relay_url=?2 AND channel_id=?3",
            params![owner, relay, channel.to_string()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if count as usize >= MAX_TASKS_PER_CHANNEL {
        return Err("task board is full for this channel; cancel or complete tasks first".into());
    }
    let mut unmet = Vec::new();
    for dependency in &dependencies {
        match get(conn, owner, relay, dependency)? {
            Some(task) if task.channel_id == channel => {
                if task.state != "completed" {
                    unmet.push(dependency.clone());
                }
            }
            _ => return Err(format!("unknown dependency task {dependency}")),
        }
    }
    let state = if !required_inputs.is_empty() || !unmet.is_empty() {
        "waiting_input"
    } else {
        "planned"
    };
    let task_id = Uuid::new_v4().to_string();
    let at = now()?;
    conn.execute(
        "INSERT INTO channel_project_tasks(task_id, identity_pubkey, relay_url, channel_id, title,
            description, state, assignee_pubkey, created_by, required_inputs, dependencies, outputs,
            return_to, revision, binding_generation, team_generation, created_at, updated_at, dispatch_state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10, '[]', ?11, 1, ?12, ?13, ?14, ?14, 'none')",
        params![
            task_id,
            owner,
            relay,
            channel.to_string(),
            title,
            description,
            state,
            actor.id(),
            json(&required_inputs)?,
            json(&dependencies)?,
            return_to,
            team.binding_generation,
            team.team_generation,
            at
        ],
    )
    .map_err(|error| error.to_string())?;
    record_event(conn, &task_id, 1, &actor.id(), "created", state)?;
    get(conn, owner, relay, &task_id)?.ok_or_else(|| "task vanished after insert".into())
}

/// A transition request. `expected_revision` fences stale callers.
pub enum Transition<'a> {
    /// Assign (or reassign) to a team member and notify them.
    Assign { assignee: &'a str },
    /// Assignee starts work.
    Start,
    /// Assignee delivers with at least one output reference.
    Deliver { outputs: Vec<OutputRef> },
    /// Assignee or coordinator reports a blocker.
    Block { blocker: &'a str },
    /// Assignee asks for inputs.
    RequestInput { inputs: Vec<String> },
    /// Owner or coordinator supplies inputs; the task returns to its assignee.
    ProvideInput,
    /// Independent reviewer takes the delivery.
    StartReview,
    /// Reviewer files a verdict.
    Review { verdict: &'a str, notes: &'a str },
    /// Owner accepts a reviewed delivery.
    Accept,
    /// Owner pauses.
    Pause,
    /// Owner resumes.
    Resume,
    /// Owner cancels.
    Cancel,
    /// Coordinator files the owner-facing final report.
    FileReport(FinalReport),
}

fn is_active(state: &str) -> bool {
    matches!(
        state,
        "planned"
            | "waiting_input"
            | "assigned"
            | "running"
            | "delivered"
            | "under_review"
            | "blocked"
            | "awaiting_owner_approval"
    )
}

/// Apply one transition atomically. Returns the updated task.
#[allow(clippy::too_many_arguments)]
pub fn transition(
    conn: &Connection,
    owner: &str,
    relay: &str,
    task_id: &str,
    expected_revision: i64,
    actor: Actor<'_>,
    team: &TeamView<'_>,
    transition: Transition<'_>,
) -> Result<Task, String> {
    let mut task = get(conn, owner, relay, task_id)?.ok_or("unknown task")?;
    if task.revision != expected_revision {
        return Err("task changed; reload the board before acting".into());
    }
    if task.binding_generation != team.binding_generation {
        return Err("project binding changed since this task was planned; cancel it or re-plan it under the current project".into());
    }
    let actor_id = actor.id();
    let is_owner = matches!(actor, Actor::Owner);
    let is_assignee =
        matches!(actor, Actor::Agent(pubkey) if task.assignee_pubkey.as_deref() == Some(pubkey));
    let is_coordinator = matches!(actor, Actor::Agent(pubkey) if team.coordinator == Some(pubkey) && team.has(pubkey, "assign_tasks"));
    let mut dispatch_target: Option<String> = None;
    let (kind, detail): (&str, String) = match transition {
        Transition::Assign { assignee } => {
            if !(is_owner || is_coordinator) {
                return Err(
                    "only the owner or the coordinator with assign_tasks may assign".into(),
                );
            }
            if team.paused {
                return Err("project dispatch is paused; resume before assigning".into());
            }
            if !team.is_member(assignee) {
                return Err("assignee must be a current project team member".into());
            }
            if matches!(task.state.as_str(), "assigned" | "running") {
                return Err(
                    "task already has an active assignee; block or cancel it before reassigning"
                        .into(),
                );
            }
            if !matches!(
                task.state.as_str(),
                "planned" | "waiting_input" | "blocked" | "delivered" | "under_review"
            ) {
                return Err(format!("cannot assign a task in state {}", task.state));
            }
            for dependency in &task.dependencies {
                let done = get(conn, owner, relay, dependency)?
                    .is_some_and(|dep| dep.state == "completed");
                if !done {
                    return Err(format!("dependency {dependency} is not completed"));
                }
            }
            if !task.required_inputs.is_empty() {
                return Err("required inputs are still outstanding; provide them first".into());
            }
            task.assignee_pubkey = Some(assignee.to_string());
            task.state = "assigned".into();
            task.blocker = None;
            dispatch_target = Some(assignee.to_string());
            ("assigned", assignee.to_string())
        }
        Transition::Start => {
            if !is_assignee {
                return Err("only the assignee may start the task".into());
            }
            if task.state != "assigned" {
                return Err(format!("cannot start a task in state {}", task.state));
            }
            if team.paused {
                return Err("project dispatch is paused".into());
            }
            task.state = "running".into();
            ("started", String::new())
        }
        Transition::Deliver { outputs } => {
            if !is_assignee {
                return Err("only the assignee may deliver".into());
            }
            if task.state != "running" {
                return Err(format!(
                    "cannot deliver a task in state {}; a delivery is recorded once",
                    task.state
                ));
            }
            if outputs.is_empty() || outputs.len() > MAX_LIST {
                return Err("a delivery needs between 1 and 32 output references".into());
            }
            for output in &outputs {
                bounded(&output.kind, "output kind")?;
                bounded(&output.reference, "output reference")?;
            }
            task.outputs = outputs;
            task.state = "delivered".into();
            dispatch_target = Some(task.return_to.clone());
            ("delivered", format!("{} output(s)", task.outputs.len()))
        }
        Transition::Block { blocker } => {
            if !(is_assignee || is_coordinator || is_owner) {
                return Err("only the assignee, coordinator, or owner may block".into());
            }
            if !matches!(task.state.as_str(), "assigned" | "running" | "under_review") {
                return Err(format!("cannot block a task in state {}", task.state));
            }
            let blocker = bounded(blocker, "blocker")?;
            task.blocker = Some(blocker.clone());
            task.state = "blocked".into();
            dispatch_target = Some(task.return_to.clone());
            ("blocked", blocker)
        }
        Transition::RequestInput { inputs } => {
            if !is_assignee {
                return Err("only the assignee may request inputs".into());
            }
            if !matches!(task.state.as_str(), "assigned" | "running") {
                return Err(format!("cannot request inputs in state {}", task.state));
            }
            let inputs = bounded_list(&inputs, "required input")?;
            if inputs.is_empty() {
                return Err("name at least one required input".into());
            }
            task.required_inputs = inputs.clone();
            task.state = "waiting_input".into();
            dispatch_target = Some(task.return_to.clone());
            ("waiting_input", inputs.join("; "))
        }
        Transition::ProvideInput => {
            if !(is_owner || is_coordinator) {
                return Err("only the owner or coordinator may mark inputs as provided".into());
            }
            if task.state != "waiting_input" {
                return Err(format!(
                    "task is not waiting for input (state {})",
                    task.state
                ));
            }
            task.required_inputs.clear();
            task.state = if task.assignee_pubkey.is_some() {
                dispatch_target = task.assignee_pubkey.clone();
                "assigned".into()
            } else {
                "planned".into()
            };
            ("input_provided", String::new())
        }
        Transition::StartReview => {
            let reviewer = match actor {
                Actor::Agent(pubkey) if team.has(pubkey, "review") => pubkey,
                _ => return Err("only an agent with the review grant may review".into()),
            };
            if task.assignee_pubkey.as_deref() == Some(reviewer) {
                return Err(
                    "review must be independent: the assignee cannot review its own delivery"
                        .into(),
                );
            }
            if task.state != "delivered" {
                return Err(format!("cannot review a task in state {}", task.state));
            }
            task.state = "under_review".into();
            ("review_started", reviewer.to_string())
        }
        Transition::Review { verdict, notes } => {
            let reviewer = match actor {
                Actor::Agent(pubkey) if team.has(pubkey, "review") => pubkey,
                _ => return Err("only an agent with the review grant may review".into()),
            };
            if task.assignee_pubkey.as_deref() == Some(reviewer) {
                return Err(
                    "review must be independent: the assignee cannot review its own delivery"
                        .into(),
                );
            }
            if task.state != "under_review" {
                return Err(format!("cannot file a review in state {}", task.state));
            }
            if !matches!(verdict, "approved" | "changes_requested") {
                return Err("verdict must be approved or changes_requested".into());
            }
            let notes = bounded(notes, "review notes")?;
            task.review = Some(ReviewRecord {
                reviewer_pubkey: reviewer.to_string(),
                verdict: verdict.to_string(),
                notes: notes.clone(),
                reviewed_at: now()?,
            });
            task.state = if verdict == "approved" {
                "awaiting_owner_approval".into()
            } else {
                "assigned".into()
            };
            dispatch_target = Some(if verdict == "approved" {
                task.return_to.clone()
            } else {
                task.assignee_pubkey
                    .clone()
                    .unwrap_or_else(|| task.return_to.clone())
            });
            ("reviewed", format!("{verdict}: {notes}"))
        }
        Transition::Accept => {
            if !is_owner {
                return Err("only the owner accepts deliveries".into());
            }
            if task.state != "awaiting_owner_approval" {
                return Err(format!(
                    "cannot accept a task in state {}; it needs a delivery and an independent approved review first",
                    task.state
                ));
            }
            if task
                .review
                .as_ref()
                .is_none_or(|review| review.verdict != "approved")
            {
                return Err("an approved independent review is required before acceptance".into());
            }
            task.state = "completed".into();
            ("accepted", String::new())
        }
        Transition::Pause => {
            if !is_owner {
                return Err("only the owner pauses tasks".into());
            }
            if !is_active(&task.state) {
                return Err(format!("cannot pause a task in state {}", task.state));
            }
            task.state_before_pause = Some(task.state.clone());
            task.state = "paused".into();
            ("paused", String::new())
        }
        Transition::Resume => {
            if !is_owner {
                return Err("only the owner resumes tasks".into());
            }
            if task.state != "paused" {
                return Err("task is not paused".into());
            }
            let previous = task
                .state_before_pause
                .take()
                .unwrap_or_else(|| "planned".into());
            // In-flight work resumes where it was; a paused running task is not
            // assumed to still be running, so it returns to `assigned` for an
            // explicit restart by its assignee.
            task.state = if previous == "running" {
                "assigned".into()
            } else {
                previous
            };
            if task.state == "assigned" {
                dispatch_target = task.assignee_pubkey.clone();
            }
            ("resumed", task.state.clone())
        }
        Transition::Cancel => {
            if !is_owner {
                return Err("only the owner cancels tasks".into());
            }
            if matches!(task.state.as_str(), "completed" | "cancelled") {
                return Err(format!("task is already {}", task.state));
            }
            task.state = "cancelled".into();
            ("cancelled", String::new())
        }
        Transition::FileReport(report) => {
            if !(is_coordinator || is_owner) {
                return Err(
                    "only the coordinator with assign_tasks or the owner may file the final report"
                        .into(),
                );
            }
            if !matches!(task.state.as_str(), "awaiting_owner_approval" | "completed") {
                return Err("a final report may only be filed after delivery and an approved independent review".into());
            }
            if task
                .review
                .as_ref()
                .is_none_or(|review| review.verdict != "approved")
            {
                return Err("a final report requires an approved independent review".into());
            }
            let mut report = report;
            for field in [
                &mut report.requested,
                &mut report.delivered,
                &mut report.verified,
                &mut report.failed_or_unverified,
                &mut report.decision_needed,
            ] {
                *field = bounded(field, "report field")?;
            }
            report.author_pubkey = actor_id.clone();
            report.filed_at = now()?;
            task.report = Some(report);
            ("report_filed", String::new())
        }
    };
    task.revision += 1;
    task.updated_at = now()?;
    if let Some(target) = dispatch_target {
        task.dispatch_state = "pending".into();
        task.dispatch_target = Some(target);
        task.dispatch_event_id = None;
        task.dispatch_attempted_at = None;
    }
    conn.execute(
        "UPDATE channel_project_tasks SET title=?4, description=?5, state=?6, state_before_pause=?7,
            assignee_pubkey=?8, required_inputs=?9, dependencies=?10, outputs=?11, return_to=?12,
            blocker=?13, review=?14, report=?15, revision=?16, updated_at=?17, dispatch_state=?18,
            dispatch_target=?19, dispatch_event_id=?20, dispatch_attempted_at=?21
         WHERE identity_pubkey=?1 AND relay_url=?2 AND task_id=?3 AND revision=?22",
        params![
            owner,
            relay,
            task_id,
            task.title,
            task.description,
            task.state,
            task.state_before_pause,
            task.assignee_pubkey,
            json(&task.required_inputs)?,
            json(&task.dependencies)?,
            json(&task.outputs)?,
            task.return_to,
            task.blocker,
            task.review.as_ref().map(json).transpose()?,
            task.report.as_ref().map(json).transpose()?,
            task.revision,
            task.updated_at,
            task.dispatch_state,
            task.dispatch_target,
            task.dispatch_event_id,
            task.dispatch_attempted_at,
            expected_revision,
        ],
    )
    .map_err(|error| error.to_string())
    .and_then(|changed| {
        if changed == 1 {
            Ok(())
        } else {
            Err("task changed concurrently; reload the board".into())
        }
    })?;
    record_event(conn, task_id, task.revision, &actor_id, kind, &detail)?;
    Ok(task)
}

/// Mark a dispatch attempt as started (before the relay publish). Idempotent per revision.
pub fn begin_dispatch(
    conn: &Connection,
    owner: &str,
    relay: &str,
    task_id: &str,
    revision: i64,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE channel_project_tasks SET dispatch_attempted_at=?4
             WHERE identity_pubkey=?1 AND relay_url=?2 AND task_id=?3 AND revision=?5
               AND dispatch_state='pending' AND dispatch_attempted_at IS NULL",
            params![owner, relay, task_id, now()?, revision],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("dispatch is not pending at this revision".into());
    }
    Ok(())
}

/// Record one dispatch attempt (or reconciliation note) in the task's audit trail.
pub fn note_dispatch(
    conn: &Connection,
    task_id: &str,
    revision: i64,
    kind: &str,
    detail: &str,
) -> Result<(), String> {
    record_event(conn, task_id, revision, "desktop", kind, detail)
}

/// Number of dispatch attempts recorded for this task revision.
pub fn dispatch_attempts(conn: &Connection, task_id: &str, revision: i64) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM channel_project_task_events
         WHERE task_id=?1 AND revision=?2 AND kind='dispatch_attempt'",
        params![task_id, revision],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

/// Confirm a dispatch with the relay event id.
pub fn confirm_dispatch(
    conn: &Connection,
    owner: &str,
    relay: &str,
    task_id: &str,
    revision: i64,
    event_id: &str,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE channel_project_tasks SET dispatch_state='sent', dispatch_event_id=?4
             WHERE identity_pubkey=?1 AND relay_url=?2 AND task_id=?3 AND revision=?5
               AND dispatch_state='pending'",
            params![owner, relay, task_id, event_id, revision],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("dispatch is not pending at this revision".into());
    }
    Ok(())
}

/// Mark a dispatch whose relay outcome is unknown. Never retried automatically.
pub fn mark_dispatch_uncertain(
    conn: &Connection,
    owner: &str,
    relay: &str,
    task_id: &str,
    revision: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE channel_project_tasks SET dispatch_state='uncertain'
         WHERE identity_pubkey=?1 AND relay_url=?2 AND task_id=?3 AND revision=?4
           AND dispatch_state='pending'",
        params![owner, relay, task_id, revision],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Owner-confirmed resolution of an uncertain dispatch: either it was observed on
/// the relay (`observed_event_id`) or the owner asks for one more attempt.
pub fn resolve_uncertain_dispatch(
    conn: &Connection,
    owner: &str,
    relay: &str,
    task_id: &str,
    revision: i64,
    observed_event_id: Option<&str>,
) -> Result<(), String> {
    let (state, event) = match observed_event_id {
        Some(event_id) => ("sent", Some(event_id.to_string())),
        None => ("pending", None),
    };
    let changed = conn
        .execute(
            "UPDATE channel_project_tasks SET dispatch_state=?4, dispatch_event_id=?5, dispatch_attempted_at=NULL
             WHERE identity_pubkey=?1 AND relay_url=?2 AND task_id=?3 AND revision=?6
               AND dispatch_state='uncertain'",
            params![owner, relay, task_id, state, event, revision],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("dispatch is not uncertain at this revision".into());
    }
    Ok(())
}

/// Task ids that any task in the channel depends on and that are not completed.
pub fn unmet_dependencies(tasks: &[Task]) -> HashSet<String> {
    let completed: HashSet<&str> = tasks
        .iter()
        .filter(|task| task.state == "completed")
        .map(|task| task.task_id.as_str())
        .collect();
    tasks
        .iter()
        .flat_map(|task| task.dependencies.iter())
        .filter(|dependency| !completed.contains(dependency.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "owner";
    const RELAY: &str = "wss://example.test";
    const COORD: &str = "coordinator";
    const WRITER: &str = "writer";
    const REVIEWER: &str = "reviewer";

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE archive_migrations(name TEXT PRIMARY KEY, applied_at INTEGER NOT NULL);",
        )
        .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn members() -> Vec<(String, Vec<String>)> {
        vec![
            (COORD.into(), vec!["read".into(), "assign_tasks".into()]),
            (WRITER.into(), vec!["read".into(), "edit".into()]),
            (REVIEWER.into(), vec!["read".into(), "review".into()]),
        ]
    }

    fn team_view(members: &[(String, Vec<String>)], paused: bool) -> TeamView<'_> {
        TeamView {
            coordinator: Some(COORD),
            members,
            paused,
            binding_generation: 1,
            team_generation: 1,
        }
    }

    fn new_task<'a>(title: &'a str) -> NewTask<'a> {
        NewTask {
            title,
            description: "do the thing",
            required_inputs: &[],
            dependencies: &[],
            return_to: COORD,
        }
    }

    #[test]
    fn full_lifecycle_requires_independent_review_before_acceptance() {
        let conn = database();
        let members = members();
        let team = team_view(&members, false);
        let channel = Uuid::new_v4();
        let task = create(
            &conn,
            OWNER,
            RELAY,
            channel,
            Actor::Agent(COORD),
            &team,
            new_task("t"),
        )
        .unwrap();
        assert_eq!(task.state, "planned");
        // A writer without assign_tasks cannot plan.
        assert!(create(
            &conn,
            OWNER,
            RELAY,
            channel,
            Actor::Agent(WRITER),
            &team,
            new_task("x")
        )
        .is_err());

        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            1,
            Actor::Agent(COORD),
            &team,
            Transition::Assign { assignee: WRITER },
        )
        .unwrap();
        assert_eq!(
            (
                t.state.as_str(),
                t.dispatch_state.as_str(),
                t.dispatch_target.as_deref()
            ),
            ("assigned", "pending", Some(WRITER))
        );
        // Duplicate assignment is refused; stale revision is refused.
        assert!(transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            2,
            Actor::Agent(COORD),
            &team,
            Transition::Assign { assignee: REVIEWER }
        )
        .is_err());
        assert!(transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            1,
            Actor::Agent(WRITER),
            &team,
            Transition::Start
        )
        .is_err());
        // Only the assignee starts; premature delivery is refused.
        assert!(transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            2,
            Actor::Agent(COORD),
            &team,
            Transition::Start
        )
        .is_err());
        assert!(transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            2,
            Actor::Agent(WRITER),
            &team,
            Transition::Deliver { outputs: vec![] }
        )
        .is_err());
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            2,
            Actor::Agent(WRITER),
            &team,
            Transition::Start,
        )
        .unwrap();
        assert_eq!(t.state, "running");
        // Owner cannot accept before delivery + review.
        assert!(transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            3,
            Actor::Owner,
            &team,
            Transition::Accept
        )
        .is_err());
        let output = OutputRef {
            kind: "file".into(),
            reference: "src/main.rs".into(),
            sha256: None,
        };
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            3,
            Actor::Agent(WRITER),
            &team,
            Transition::Deliver {
                outputs: vec![output],
            },
        )
        .unwrap();
        assert_eq!(
            (t.state.as_str(), t.dispatch_target.as_deref()),
            ("delivered", Some(COORD))
        );
        // Second delivery is refused (recorded once).
        assert!(transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            4,
            Actor::Agent(WRITER),
            &team,
            Transition::Deliver { outputs: vec![] }
        )
        .is_err());
        // The assignee cannot review its own work; the coordinator lacks the review grant.
        assert!(transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            4,
            Actor::Agent(WRITER),
            &team,
            Transition::StartReview
        )
        .is_err());
        assert!(transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            4,
            Actor::Agent(COORD),
            &team,
            Transition::StartReview
        )
        .is_err());
        // Final report before review is refused.
        let report = FinalReport {
            requested: "r".into(),
            delivered: "d".into(),
            verified: "v".into(),
            failed_or_unverified: "f".into(),
            decision_needed: "n".into(),
            author_pubkey: String::new(),
            filed_at: 0,
        };
        assert!(transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            4,
            Actor::Agent(COORD),
            &team,
            Transition::FileReport(report.clone())
        )
        .is_err());
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            4,
            Actor::Agent(REVIEWER),
            &team,
            Transition::StartReview,
        )
        .unwrap();
        assert_eq!(t.state, "under_review");
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            5,
            Actor::Agent(REVIEWER),
            &team,
            Transition::Review {
                verdict: "approved",
                notes: "lgtm",
            },
        )
        .unwrap();
        assert_eq!(t.state, "awaiting_owner_approval");
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            6,
            Actor::Agent(COORD),
            &team,
            Transition::FileReport(report),
        )
        .unwrap();
        assert_eq!(t.report.as_ref().unwrap().author_pubkey, COORD);
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            7,
            Actor::Owner,
            &team,
            Transition::Accept,
        )
        .unwrap();
        assert_eq!(t.state, "completed");
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM channel_project_task_events WHERE task_id=?1",
                [&task.task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 8);
    }

    #[test]
    fn inputs_dependencies_pause_and_binding_changes_are_fenced() {
        let conn = database();
        let members = members();
        let team = team_view(&members, false);
        let channel = Uuid::new_v4();
        let first = create(
            &conn,
            OWNER,
            RELAY,
            channel,
            Actor::Owner,
            &team,
            new_task("first"),
        )
        .unwrap();
        let deps = vec![first.task_id.clone()];
        let second = create(
            &conn,
            OWNER,
            RELAY,
            channel,
            Actor::Owner,
            &team,
            NewTask {
                title: "second",
                description: "after first",
                required_inputs: &[],
                dependencies: &deps,
                return_to: "owner",
            },
        )
        .unwrap();
        assert_eq!(second.state, "waiting_input");
        assert!(
            transition(
                &conn,
                OWNER,
                RELAY,
                &second.task_id,
                1,
                Actor::Owner,
                &team,
                Transition::Assign { assignee: WRITER }
            )
            .is_err(),
            "unmet dependency blocks assignment"
        );
        assert!(create(
            &conn,
            OWNER,
            RELAY,
            channel,
            Actor::Owner,
            &team,
            NewTask {
                title: "bad",
                description: "x",
                required_inputs: &[],
                dependencies: &["nope".into()],
                return_to: "owner",
            }
        )
        .is_err());

        let paused = team_view(&members, true);
        assert!(
            transition(
                &conn,
                OWNER,
                RELAY,
                &first.task_id,
                1,
                Actor::Owner,
                &paused,
                Transition::Assign { assignee: WRITER }
            )
            .is_err(),
            "paused dispatch refuses assignment"
        );
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &first.task_id,
            1,
            Actor::Owner,
            &team,
            Transition::Assign { assignee: WRITER },
        )
        .unwrap();
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &t.task_id,
            2,
            Actor::Agent(WRITER),
            &team,
            Transition::Start,
        )
        .unwrap();
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &t.task_id,
            3,
            Actor::Agent(WRITER),
            &team,
            Transition::RequestInput {
                inputs: vec!["API key name".into()],
            },
        )
        .unwrap();
        assert_eq!(
            (t.state.as_str(), t.dispatch_target.as_deref()),
            ("waiting_input", Some(COORD))
        );
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &t.task_id,
            4,
            Actor::Owner,
            &team,
            Transition::ProvideInput,
        )
        .unwrap();
        assert_eq!(
            (t.state.as_str(), t.dispatch_target.as_deref()),
            ("assigned", Some(WRITER))
        );
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &t.task_id,
            5,
            Actor::Owner,
            &team,
            Transition::Pause,
        )
        .unwrap();
        assert_eq!(t.state, "paused");
        assert!(transition(
            &conn,
            OWNER,
            RELAY,
            &t.task_id,
            6,
            Actor::Agent(WRITER),
            &team,
            Transition::Start
        )
        .is_err());
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &t.task_id,
            6,
            Actor::Owner,
            &team,
            Transition::Resume,
        )
        .unwrap();
        assert_eq!(t.state, "assigned");
        let rebound = TeamView {
            binding_generation: 2,
            ..team_view(&members, false)
        };
        assert!(
            transition(
                &conn,
                OWNER,
                RELAY,
                &t.task_id,
                7,
                Actor::Owner,
                &rebound,
                Transition::Cancel
            )
            .is_err(),
            "a binding change never silently redirects a task"
        );
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &t.task_id,
            7,
            Actor::Owner,
            &team,
            Transition::Cancel,
        )
        .unwrap();
        assert_eq!(t.state, "cancelled");
        assert!(list(&conn, OWNER, RELAY, channel).unwrap().len() == 2);
        assert!(list(&conn, "other", RELAY, channel).unwrap().is_empty());
        assert_eq!(
            unmet_dependencies(&list(&conn, OWNER, RELAY, channel).unwrap()).len(),
            1
        );
    }

    #[test]
    fn dispatch_receipts_survive_and_never_auto_retry_when_uncertain() {
        let conn = database();
        let members = members();
        let team = team_view(&members, false);
        let channel = Uuid::new_v4();
        let task = create(
            &conn,
            OWNER,
            RELAY,
            channel,
            Actor::Owner,
            &team,
            new_task("t"),
        )
        .unwrap();
        let t = transition(
            &conn,
            OWNER,
            RELAY,
            &task.task_id,
            1,
            Actor::Owner,
            &team,
            Transition::Assign { assignee: WRITER },
        )
        .unwrap();
        assert_eq!(dispatch_backlog(&conn, OWNER, RELAY).unwrap().len(), 1);
        begin_dispatch(&conn, OWNER, RELAY, &t.task_id, t.revision).unwrap();
        assert!(
            begin_dispatch(&conn, OWNER, RELAY, &t.task_id, t.revision).is_err(),
            "one attempt per revision"
        );
        // Crash after the side effect: the owner sees `uncertain`, nothing retries.
        mark_dispatch_uncertain(&conn, OWNER, RELAY, &t.task_id, t.revision).unwrap();
        let stored = get(&conn, OWNER, RELAY, &t.task_id).unwrap().unwrap();
        assert_eq!(stored.dispatch_state, "uncertain");
        assert!(confirm_dispatch(&conn, OWNER, RELAY, &t.task_id, t.revision, "evt").is_err());
        resolve_uncertain_dispatch(
            &conn,
            OWNER,
            RELAY,
            &t.task_id,
            t.revision,
            Some("evt-observed"),
        )
        .unwrap();
        let stored = get(&conn, OWNER, RELAY, &t.task_id).unwrap().unwrap();
        assert_eq!(
            (
                stored.dispatch_state.as_str(),
                stored.dispatch_event_id.as_deref()
            ),
            ("sent", Some("evt-observed"))
        );
        assert!(dispatch_backlog(&conn, OWNER, RELAY).unwrap().is_empty());
    }
}
