import { invokeTauri } from "./tauri";

export type ProjectCapability =
  | "read"
  | "edit"
  | "run_checks"
  | "manage_team"
  | "assign_tasks"
  | "review"
  | "create_private_channels"
  | "commit_merge";

export type ProjectRole =
  | "coordinator"
  | "manager"
  | "writer"
  | "tester"
  | "reviewer"
  | "member";

/** Fallback vocabularies when a status predates the shared lists. */
export const DEFAULT_PROJECT_ROLES: ProjectRole[] = [
  "coordinator",
  "manager",
  "writer",
  "tester",
  "reviewer",
  "member",
];
export const DEFAULT_PROJECT_CAPABILITIES: ProjectCapability[] = [
  "read",
  "edit",
  "run_checks",
  "manage_team",
  "assign_tasks",
  "review",
  "create_private_channels",
  "commit_merge",
];

export type Confinement = "os_sandbox" | "harness_gate" | "unconfined";

export type ProjectGateProfile = {
  harness: string;
  gate_mode: string | null;
  write_confinement: Confinement;
  read_confinement: Confinement;
  command_confinement: Confinement;
  network_blocked: boolean;
  evidence: string;
};

export type ProjectEnforcement =
  | { kind: "native_sandbox" }
  | { kind: "gate"; profile: ProjectGateProfile }
  | { kind: "unverified"; reason: string };

export type ProjectTeamMember = {
  pubkey: string;
  display_name: string;
  role: ProjectRole;
  capabilities: ProjectCapability[];
};

export type ReadinessCheck = {
  id: string;
  status: "ok" | "warn" | "fail" | "unknown";
  message: string;
  recovery?: string;
};

export type ReadinessState =
  | "conversation_only"
  | "selected"
  | "attached"
  | "runtime_configured"
  | "access_verified"
  | "execution_authorized"
  | "task_running";

export type ProjectReadiness = {
  state: ReadinessState;
  checked_at: number;
  checks: ReadinessCheck[];
  blockers: string[];
  git: {
    branch: string | null;
    revision: string | null;
    dirty: boolean | null;
    verified_at: number | null;
  };
  last_inspection_at: number | null;
  pending_approvals: number;
  running_tasks: number;
};

export type AvailableProjectAgent = {
  pubkey: string;
  display_name: string;
  runtime_id: string | null;
  local: boolean;
  enforcement?: ProjectEnforcement;
};

export type ProjectTeamStatus = {
  generation: number;
  binding_generation: number;
  coordinator_pubkey: string | null;
  members: ProjectTeamMember[];
  available_agents: AvailableProjectAgent[];
  paused: boolean;
  approved_checks?: string[];
  readiness?: ProjectReadiness;
  execution_blockers: string[];
  roles?: ProjectRole[];
  capabilities?: ProjectCapability[];
};

export type ProjectTeamScope = {
  channelId: string;
  expectedRelay: string;
  expectedOwner: string;
};

export type ProjectTeamDraft = {
  generation: number;
  binding_generation: number;
  coordinator_pubkey: string | null;
  members: ProjectTeamMember[];
  paused: boolean;
  approved_checks: string[];
};

export function getChannelProjectTeam(scope: ProjectTeamScope) {
  return invokeTauri<ProjectTeamStatus>("get_channel_project_team", scope);
}

export function setChannelProjectTeam(
  scope: ProjectTeamScope,
  draft: ProjectTeamDraft,
) {
  return invokeTauri<ProjectTeamStatus>("set_channel_project_team", {
    ...scope,
    expectedGeneration: draft.generation,
    expectedBindingGeneration: draft.binding_generation,
    coordinatorPubkey: draft.coordinator_pubkey,
    members: draft.members.map(({ pubkey, role, capabilities }) => ({
      pubkey,
      role,
      capabilities,
    })),
    paused: draft.paused,
    approvedChecks: draft.approved_checks,
  });
}

/** Pause or resume dispatch without touching grants or running processes. */
export function setChannelProjectPaused(
  scope: ProjectTeamScope,
  expectedGeneration: number,
  paused: boolean,
) {
  return invokeTauri<ProjectTeamStatus>("set_channel_project_paused", {
    ...scope,
    expectedGeneration,
    paused,
  });
}

export type ProjectCoordinatorStartup = {
  status: "started" | "blocked";
  transport: "native" | "codex_reasoning_broker_v1" | "unsupported";
  binding_generation: number;
  team_generation: number;
  coordinator_pubkey: string;
  requires_explicit_opt_in: boolean;
  authentication_required: boolean;
  blockers: { code: string; message: string }[];
  runtime_receipt: { pid: number; started_at: number } | null;
  execution_ready: false;
};

export function startChannelProjectCoordinator(
  scope: ProjectTeamScope,
  bindingGeneration: number,
  teamGeneration: number,
  coordinatorPubkey: string,
  explicitBridgeOptIn: boolean,
) {
  return invokeTauri<ProjectCoordinatorStartup>(
    "start_channel_project_coordinator",
    {
      ...scope,
      expectedBindingGeneration: bindingGeneration,
      expectedTeamGeneration: teamGeneration,
      coordinatorPubkey,
      explicitBridgeOptIn,
    },
  );
}

// ── Approval queue ──────────────────────────────────────────────────────────

export type ApprovalRequest = {
  request_id: string;
  channel_id: string;
  agent_pubkey: string;
  category:
    | "execute"
    | "edit"
    | "read"
    | "fetch"
    | "commit"
    | "publish"
    | "channel"
    | "other";
  summary: string;
  reason: string;
  state: "pending" | "approved" | "denied";
  requested_at: number;
  decided_at: number | null;
  decision_note: string | null;
  binding_generation: number;
  team_generation: number;
};

export function listProjectApprovalRequests(
  scope: ProjectTeamScope,
): Promise<ApprovalRequest[]> {
  return invokeTauri<ApprovalRequest[]>("list_project_approval_requests", {
    expectedRelay: scope.expectedRelay,
    expectedOwner: scope.expectedOwner,
    channelId: scope.channelId,
  });
}

/**
 * Every request still waiting on the owner, across all of the community's
 * channels. The same command as {@link listProjectApprovalRequests}, asked
 * for no channel in particular.
 */
export function listPendingApprovalRequests(scope: {
  expectedRelay: string;
  expectedOwner: string;
}): Promise<ApprovalRequest[]> {
  return invokeTauri<ApprovalRequest[]>("list_project_approval_requests", {
    expectedRelay: scope.expectedRelay,
    expectedOwner: scope.expectedOwner,
    channelId: null,
  }).then((requests) =>
    requests.filter((request) => request.state === "pending"),
  );
}

export function decideProjectApprovalRequest(
  scope: ProjectTeamScope,
  input: {
    requestId: string;
    approve: boolean;
    note?: string;
    addToApprovedChecks: boolean;
    expectedTeamGeneration: number | null;
  },
) {
  return invokeTauri<ApprovalRequest>("decide_project_approval_request", {
    expectedRelay: scope.expectedRelay,
    expectedOwner: scope.expectedOwner,
    requestId: input.requestId,
    approve: input.approve,
    note: input.note ?? null,
    addToApprovedChecks: input.addToApprovedChecks,
    expectedTeamGeneration: input.expectedTeamGeneration,
  });
}

// ── Task board ──────────────────────────────────────────────────────────────

export type TaskState =
  | "planned"
  | "waiting_input"
  | "assigned"
  | "running"
  | "delivered"
  | "under_review"
  | "blocked"
  | "awaiting_owner_approval"
  | "completed"
  | "paused"
  | "cancelled";

export const TASK_STATES: TaskState[] = [
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

export type TaskOutput = { kind: string; reference: string; sha256?: string };

export type TaskReview = {
  reviewer_pubkey: string;
  verdict: "approved" | "changes_requested";
  notes: string;
  reviewed_at: number;
};

export type TaskReport = {
  requested: string;
  delivered: string;
  verified: string;
  failed_or_unverified: string;
  decision_needed: string;
  author_pubkey: string;
  filed_at: number;
};

export type ProjectTask = {
  task_id: string;
  channel_id: string;
  title: string;
  description: string;
  state: TaskState;
  state_before_pause: string | null;
  assignee_pubkey: string | null;
  created_by: string;
  required_inputs: string[];
  dependencies: string[];
  outputs: TaskOutput[];
  return_to: string;
  blocker: string | null;
  review: TaskReview | null;
  report: TaskReport | null;
  revision: number;
  binding_generation: number;
  team_generation: number;
  created_at: number;
  updated_at: number;
  dispatch_state: "none" | "pending" | "sent" | "uncertain";
  dispatch_target: string | null;
  dispatch_event_id: string | null;
  dispatch_attempted_at: number | null;
};

export function listChannelProjectTasks(scope: ProjectTeamScope) {
  return invokeTauri<ProjectTask[]>("list_channel_project_tasks", scope);
}

export function createChannelProjectTask(
  scope: ProjectTeamScope,
  input: {
    title: string;
    description: string;
    requiredInputs?: string[];
    dependencies?: string[];
    returnTo?: string | null;
    assigneePubkey?: string | null;
  },
) {
  return invokeTauri<ProjectTask>("create_channel_project_task", {
    ...scope,
    input: {
      title: input.title,
      description: input.description,
      requiredInputs: input.requiredInputs ?? [],
      dependencies: input.dependencies ?? [],
      returnTo: input.returnTo ?? null,
      assigneePubkey: input.assigneePubkey ?? null,
    },
  });
}

export type OwnerTaskAction =
  | { action: "assign"; assignee_pubkey: string }
  | { action: "provide_input" }
  | { action: "block"; blocker: string }
  | { action: "accept" }
  | { action: "pause" }
  | { action: "resume" }
  | { action: "cancel" };

export function transitionChannelProjectTask(
  scope: ProjectTeamScope,
  taskId: string,
  expectedRevision: number,
  action: OwnerTaskAction,
) {
  return invokeTauri<ProjectTask>("transition_channel_project_task", {
    ...scope,
    taskId,
    expectedRevision,
    action,
  });
}

export function resolveChannelProjectTaskDispatch(
  scope: ProjectTeamScope,
  taskId: string,
  expectedRevision: number,
  observedEventId: string | null,
) {
  return invokeTauri<ProjectTask>("resolve_channel_project_task_dispatch", {
    expectedRelay: scope.expectedRelay,
    expectedOwner: scope.expectedOwner,
    taskId,
    expectedRevision,
    observedEventId,
  });
}

export type DispatchSummary = {
  published: number;
  in_app: number;
  reconciled: number;
  uncertain: number;
  deferred: number;
};

export function dispatchChannelProjectTasks(scope: ProjectTeamScope) {
  return invokeTauri<DispatchSummary>("dispatch_channel_project_tasks", {
    expectedRelay: scope.expectedRelay,
    expectedOwner: scope.expectedOwner,
  });
}

// ── Handoff and change review ───────────────────────────────────────────────

export type HandoffRecord = {
  source: "file" | "imported";
  label: string;
  text: string;
  captured_at: number;
  binding_generation: number;
  git_revision: string | null;
  dirty_at_capture: boolean | null;
  previous_writer_released_at: number | null;
};

export type ChangedPath = { status: string; path: string };

export type HandoffInspection = {
  channel_id: string;
  binding_generation: number;
  checked_at: number;
  record: HandoffRecord | null;
  probed: string[];
  unfinished_items: string[];
  referenced_paths: string[];
  live: {
    branch: string | null;
    revision: string | null;
    dirty: boolean | null;
    changed_paths: ChangedPath[];
  };
  foreign_processes: { pid: number; command: string; cwd: string }[];
  foreign_scan_complete: boolean;
};

export function inspectChannelProjectHandoff(
  scope: ProjectTeamScope,
  expectedBindingGeneration: number,
) {
  return invokeTauri<HandoffInspection>("inspect_channel_project_handoff", {
    ...scope,
    expectedBindingGeneration,
  });
}

export function importChannelProjectHandoffSummary(
  scope: ProjectTeamScope,
  expectedBindingGeneration: number,
  label: string,
  text: string,
) {
  return invokeTauri<HandoffRecord>("import_channel_project_handoff_summary", {
    ...scope,
    expectedBindingGeneration,
    label,
    text,
  });
}

export function acknowledgeChannelProjectPreviousWriterRelease(
  scope: ProjectTeamScope,
) {
  return invokeTauri<HandoffRecord>(
    "acknowledge_channel_project_previous_writer_release",
    { ...scope, confirmed: true },
  );
}

export type ProjectChangeReview = {
  channel_id: string;
  binding_generation: number;
  checked_at: number;
  changes: {
    actual_cwd: string;
    repository: boolean;
    changed_paths: ChangedPath[];
    paths_truncated: boolean;
    diff_stat: string;
    diff: string;
    diff_truncated: boolean;
    runtime: string;
  };
  overlaps: { path: string; task_ids: string[] }[];
  edit_receipts: {
    operation_id: string;
    relative_path: string;
    before_sha256: string;
    after_sha256: string;
    state: string;
    created_at: number;
  }[];
  awaiting: string[];
};

export function getChannelProjectChanges(
  scope: ProjectTeamScope,
  expectedBindingGeneration: number,
) {
  return invokeTauri<ProjectChangeReview>("get_channel_project_changes", {
    ...scope,
    expectedBindingGeneration,
  });
}
