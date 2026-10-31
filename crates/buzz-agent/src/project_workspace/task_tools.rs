//! Task board tools exposed to agents.
//!
//! One implementation serves two transports: the native `buzz-agent` project
//! session (fixed MCP-less tools) and the stdio MCP server that harness-gated
//! runtimes (Claude Code, Codex) connect to. Both establish the acting identity
//! from the launch policy — never from tool arguments — and re-check the
//! project authority on every call before touching the board.

use super::{tasks, teams, ProjectPolicy};
use buzz_core::local_workspace::ProjectSessionScope;
use rusqlite::TransactionBehavior;
use serde::Deserialize;
use serde_json::{json, Value};

/// A tool definition in the shape both transports understand.
#[derive(Clone, Debug)]
pub struct TaskToolDef {
    /// Tool name, always `project__task_*`.
    pub name: String,
    /// Model-facing description.
    pub description: String,
    /// JSON schema for the arguments.
    pub input_schema: Value,
}

fn def(name: &str, description: &str, properties: Value, required: &[&str]) -> TaskToolDef {
    TaskToolDef {
        name: name.into(),
        description: description.into(),
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }),
    }
}

/// Tool definitions available to an agent with the given capabilities.
pub fn definitions(capabilities: &[String]) -> Vec<TaskToolDef> {
    let has = |capability: &str| capabilities.iter().any(|grant| grant == capability);
    let id = json!({"type": "string"});
    let revision = json!({"type": "integer"});
    let mut tools = vec![
        def(
            "project__task_list",
            "List this channel project's task board: every task with its state, assignee, revision, inputs, outputs, blocker and review. Read this before acting; state advances only through the task tools, never through chat text.",
            json!({}),
            &[],
        ),
        def(
            "project__task_get",
            "Read one task in full, including its final report if filed.",
            json!({"task_id": id}),
            &["task_id"],
        ),
        def(
            "project__task_start",
            "Start a task assigned to you (state assigned → running). Requires the current revision.",
            json!({"task_id": id, "expected_revision": revision}),
            &["task_id", "expected_revision"],
        ),
        def(
            "project__task_deliver",
            "Deliver a running task you own with one to thirty-two output references (file paths, edit receipt ids, message event ids, check names or notes). A delivery is recorded once; it is not review or acceptance. The return destination is notified by the desktop.",
            json!({
                "task_id": id,
                "expected_revision": revision,
                "outputs": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"type": "object", "properties": {"kind": {"type": "string", "enum": ["file", "edit_receipt", "message", "note", "check"]}, "reference": {"type": "string"}, "sha256": {"type": "string"}}, "required": ["kind", "reference"], "additionalProperties": false}}
            }),
            &["task_id", "expected_revision", "outputs"],
        ),
        def(
            "project__task_block",
            "Record a blocker on a task you own (or coordinate). The return destination is notified.",
            json!({"task_id": id, "expected_revision": revision, "blocker": {"type": "string"}}),
            &["task_id", "expected_revision", "blocker"],
        ),
        def(
            "project__task_request_input",
            "Ask for inputs you need before continuing a task you own (state → waiting_input). Name each input precisely.",
            json!({"task_id": id, "expected_revision": revision, "inputs": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"type": "string"}}}),
            &["task_id", "expected_revision", "inputs"],
        ),
    ];
    if has("assign_tasks") {
        tools.push(def(
            "project__task_create",
            "Plan a bounded task on the board. Requires the assign_tasks grant. Dependencies must be existing task ids; return_to is the identity that receives the delivery (default: you).",
            json!({
                "title": {"type": "string"},
                "description": {"type": "string"},
                "required_inputs": {"type": "array", "maxItems": 32, "items": {"type": "string"}},
                "dependencies": {"type": "array", "maxItems": 32, "items": {"type": "string"}},
                "return_to": {"type": "string"}
            }),
            &["title", "description"],
        ));
        tools.push(def(
            "project__task_assign",
            "Assign (or reassign) a task to a current team member. Requires assign_tasks and refuses a task that already has an active assignee, unmet dependencies or outstanding inputs. The assignee is notified by the desktop.",
            json!({"task_id": id, "expected_revision": revision, "assignee_pubkey": {"type": "string"}}),
            &["task_id", "expected_revision", "assignee_pubkey"],
        ));
        tools.push(def(
            "project__task_provide_input",
            "Mark a waiting task's inputs as provided (they must be in the channel or the task description); the assignee is notified.",
            json!({"task_id": id, "expected_revision": revision}),
            &["task_id", "expected_revision"],
        ));
        tools.push(def(
            "project__task_report",
            "File the owner-facing final report for a task after its independent review was approved. Five fields, each required: what was requested, what was actually delivered, what was verified (and how), what failed or remains unverified, and what decision or approval is needed next. Write the report in clear Arabic; put necessary English terms on their own separate lines and continue Arabic on a new line.",
            json!({
                "task_id": id,
                "expected_revision": revision,
                "requested": {"type": "string"},
                "delivered": {"type": "string"},
                "verified": {"type": "string"},
                "failed_or_unverified": {"type": "string"},
                "decision_needed": {"type": "string"}
            }),
            &["task_id", "expected_revision", "requested", "delivered", "verified", "failed_or_unverified", "decision_needed"],
        ));
    }
    if has("review") {
        tools.push(def(
            "project__task_review",
            "Independently review a delivered task you did not work on. Verdict `approved` sends it to the owner for acceptance; `changes_requested` returns it to the assignee with your notes. Requires the review grant.",
            json!({"task_id": id, "expected_revision": revision, "verdict": {"type": "string", "enum": ["approved", "changes_requested"]}, "notes": {"type": "string"}}),
            &["task_id", "expected_revision", "verdict", "notes"],
        ));
    }
    tools
}

/// Whether `name` is a task tool.
pub fn is_task_tool(name: &str) -> bool {
    name.starts_with("project__task_")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskRef {
    task_id: String,
    expected_revision: i64,
}

fn compact(task: &tasks::Task) -> Value {
    json!({
        "task_id": task.task_id,
        "title": task.title,
        "state": task.state,
        "revision": task.revision,
        "assignee_pubkey": task.assignee_pubkey,
        "return_to": task.return_to,
        "required_inputs": task.required_inputs,
        "dependencies": task.dependencies,
        "outputs": task.outputs,
        "blocker": task.blocker,
        "review": task.review,
        "has_report": task.report.is_some(),
        "updated_at": task.updated_at,
    })
}

/// Execute one task tool for the agent named by `policy`, under `scope`.
///
/// The authority transaction stays open for the whole operation: the scope is
/// re-resolved against the store, the team is loaded fresh, and the transition
/// is applied in the same transaction, so a concurrent grant change or binding
/// change cannot slip between the check and the effect.
pub fn call(
    policy: &ProjectPolicy,
    scope: &ProjectSessionScope,
    name: &str,
    arguments: &Value,
) -> Result<String, String> {
    let mut conn = policy.open()?;
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let current = policy
        .resolve(&transaction, &scope.channel_id.to_string(), "read")?
        .ok_or("project was detached")?;
    if &current != scope {
        return Err("project session is stale; create a new authorized session".into());
    }
    let team = teams::load(
        &transaction,
        &scope.owner_pubkey,
        &scope.relay_url,
        scope.channel_id,
    )?;
    let members: Vec<(String, Vec<String>)> = team
        .members
        .iter()
        .map(|member| (member.pubkey.clone(), member.capabilities.clone()))
        .collect();
    let view = tasks::TeamView {
        coordinator: team.coordinator_pubkey.as_deref(),
        members: &members,
        paused: team.paused,
        binding_generation: scope.binding_generation,
        team_generation: team.generation,
    };
    let actor = tasks::Actor::Agent(&policy.agent_pubkey);
    let owner = scope.owner_pubkey.as_str();
    let relay = scope.relay_url.as_str();
    let channel = scope.channel_id;
    let parse = |what: &str| -> Result<TaskRef, String> {
        serde_json::from_value(arguments.clone()).map_err(|_| format!("invalid {what} arguments"))
    };
    let transition = |reference: TaskRef, transition: tasks::Transition<'_>| {
        let task =
            tasks::get(&transaction, owner, relay, &reference.task_id)?.ok_or("unknown task")?;
        if task.channel_id != channel {
            return Err("task belongs to another channel project".to_string());
        }
        tasks::transition(
            &transaction,
            owner,
            relay,
            &reference.task_id,
            reference.expected_revision,
            actor,
            &view,
            transition,
        )
    };
    let result = match name {
        "project__task_list" => {
            let listed = tasks::list(&transaction, owner, relay, channel)?;
            json!({"tasks": listed.iter().map(compact).collect::<Vec<_>>(), "paused": team.paused, "team_generation": team.generation})
        }
        "project__task_get" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                task_id: String,
            }
            let args: Args = serde_json::from_value(arguments.clone())
                .map_err(|_| "invalid task get arguments")?;
            let task = tasks::get(&transaction, owner, relay, &args.task_id)?
                .filter(|task| task.channel_id == channel)
                .ok_or("unknown task")?;
            serde_json::to_value(&task).map_err(|error| error.to_string())?
        }
        "project__task_create" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                title: String,
                description: String,
                #[serde(default)]
                required_inputs: Vec<String>,
                #[serde(default)]
                dependencies: Vec<String>,
                #[serde(default)]
                return_to: Option<String>,
            }
            let args: Args = serde_json::from_value(arguments.clone())
                .map_err(|_| "invalid task create arguments")?;
            let return_to = args
                .return_to
                .clone()
                .unwrap_or_else(|| policy.agent_pubkey.clone());
            let task = tasks::create(
                &transaction,
                owner,
                relay,
                channel,
                actor,
                &view,
                tasks::NewTask {
                    title: &args.title,
                    description: &args.description,
                    required_inputs: &args.required_inputs,
                    dependencies: &args.dependencies,
                    return_to: &return_to,
                },
            )?;
            compact(&task)
        }
        "project__task_assign" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                task_id: String,
                expected_revision: i64,
                assignee_pubkey: String,
            }
            let args: Args = serde_json::from_value(arguments.clone())
                .map_err(|_| "invalid task assign arguments")?;
            compact(&transition(
                TaskRef {
                    task_id: args.task_id,
                    expected_revision: args.expected_revision,
                },
                tasks::Transition::Assign {
                    assignee: &args.assignee_pubkey,
                },
            )?)
        }
        "project__task_start" => {
            compact(&transition(parse("task start")?, tasks::Transition::Start)?)
        }
        "project__task_deliver" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                task_id: String,
                expected_revision: i64,
                outputs: Vec<tasks::OutputRef>,
            }
            let args: Args = serde_json::from_value(arguments.clone())
                .map_err(|_| "invalid task deliver arguments")?;
            compact(&transition(
                TaskRef {
                    task_id: args.task_id,
                    expected_revision: args.expected_revision,
                },
                tasks::Transition::Deliver {
                    outputs: args.outputs,
                },
            )?)
        }
        "project__task_block" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                task_id: String,
                expected_revision: i64,
                blocker: String,
            }
            let args: Args = serde_json::from_value(arguments.clone())
                .map_err(|_| "invalid task block arguments")?;
            compact(&transition(
                TaskRef {
                    task_id: args.task_id,
                    expected_revision: args.expected_revision,
                },
                tasks::Transition::Block {
                    blocker: &args.blocker,
                },
            )?)
        }
        "project__task_request_input" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                task_id: String,
                expected_revision: i64,
                inputs: Vec<String>,
            }
            let args: Args = serde_json::from_value(arguments.clone())
                .map_err(|_| "invalid task input arguments")?;
            compact(&transition(
                TaskRef {
                    task_id: args.task_id,
                    expected_revision: args.expected_revision,
                },
                tasks::Transition::RequestInput {
                    inputs: args.inputs,
                },
            )?)
        }
        "project__task_provide_input" => compact(&transition(
            parse("task provide input")?,
            tasks::Transition::ProvideInput,
        )?),
        "project__task_review" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                task_id: String,
                expected_revision: i64,
                verdict: String,
                notes: String,
            }
            let args: Args = serde_json::from_value(arguments.clone())
                .map_err(|_| "invalid task review arguments")?;
            let current = tasks::get(&transaction, owner, relay, &args.task_id)?
                .filter(|task| task.channel_id == channel)
                .ok_or("unknown task")?;
            let mut revision = args.expected_revision;
            if current.state == "delivered" {
                let started = transition(
                    TaskRef {
                        task_id: args.task_id.clone(),
                        expected_revision: revision,
                    },
                    tasks::Transition::StartReview,
                )?;
                revision = started.revision;
            }
            compact(&transition(
                TaskRef {
                    task_id: args.task_id,
                    expected_revision: revision,
                },
                tasks::Transition::Review {
                    verdict: &args.verdict,
                    notes: &args.notes,
                },
            )?)
        }
        "project__task_report" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                task_id: String,
                expected_revision: i64,
                requested: String,
                delivered: String,
                verified: String,
                failed_or_unverified: String,
                decision_needed: String,
            }
            let args: Args = serde_json::from_value(arguments.clone())
                .map_err(|_| "invalid task report arguments")?;
            compact(&transition(
                TaskRef {
                    task_id: args.task_id,
                    expected_revision: args.expected_revision,
                },
                tasks::Transition::FileReport(tasks::FinalReport {
                    requested: args.requested,
                    delivered: args.delivered,
                    verified: args.verified,
                    failed_or_unverified: args.failed_or_unverified,
                    decision_needed: args.decision_needed,
                    author_pubkey: String::new(),
                    filed_at: 0,
                }),
            )?)
        }
        _ => return Err("unsupported task tool".into()),
    };
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(result.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_tool_definitions_follow_grants() {
        let names = |caps: &[&str]| {
            definitions(&caps.iter().map(|c| c.to_string()).collect::<Vec<_>>())
                .into_iter()
                .map(|tool| tool.name)
                .collect::<Vec<_>>()
        };
        let base = names(&["read"]);
        assert!(base.contains(&"project__task_list".to_string()));
        assert!(base.contains(&"project__task_deliver".to_string()));
        assert!(!base.contains(&"project__task_assign".to_string()));
        assert!(!base.contains(&"project__task_review".to_string()));
        let coordinator = names(&["read", "assign_tasks"]);
        assert!(coordinator.contains(&"project__task_assign".to_string()));
        assert!(coordinator.contains(&"project__task_report".to_string()));
        let reviewer = names(&["read", "review"]);
        assert!(reviewer.contains(&"project__task_review".to_string()));
        assert!(is_task_tool("project__task_get"));
        assert!(!is_task_tool("project__read_text"));
    }
}
