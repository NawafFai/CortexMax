import type {
  OwnerTaskAction,
  ProjectTask,
  ReadinessState,
  TaskState,
} from "@/shared/api/channelProjectTeam";

/** Owner actions the board offers for a task in `state`. */
export function ownerActionsFor(
  task: Pick<ProjectTask, "state" | "assignee_pubkey" | "required_inputs">,
): OwnerTaskAction["action"][] {
  const state: TaskState = task.state;
  switch (state) {
    case "planned":
      return ["assign", "pause", "cancel"];
    case "waiting_input":
      return ["provide_input", "pause", "cancel"];
    case "assigned":
      return ["block", "pause", "cancel"];
    case "running":
      return ["block", "pause", "cancel"];
    case "delivered":
      return ["assign", "pause", "cancel"];
    case "under_review":
      return ["block", "pause", "cancel"];
    case "blocked":
      return ["assign", "pause", "cancel"];
    case "awaiting_owner_approval":
      return ["accept", "pause", "cancel"];
    case "paused":
      return ["resume", "cancel"];
    case "completed":
    case "cancelled":
      return [];
  }
}

/** Board columns in reading order, with the states each collects. */
export const BOARD_COLUMNS: {
  id: string;
  label: string;
  states: TaskState[];
}[] = [
  { id: "queued", label: "Queued", states: ["planned", "waiting_input"] },
  { id: "active", label: "In progress", states: ["assigned", "running"] },
  {
    id: "review",
    label: "Delivered / review",
    states: ["delivered", "under_review", "awaiting_owner_approval"],
  },
  { id: "held", label: "Blocked / paused", states: ["blocked", "paused"] },
  {
    id: "done",
    label: "Completed / cancelled",
    states: ["completed", "cancelled"],
  },
];

export function groupTasks(tasks: ProjectTask[]) {
  return BOARD_COLUMNS.map((column) => ({
    ...column,
    tasks: tasks.filter((task) => column.states.includes(task.state)),
  }));
}

/** Plain-language meaning of a readiness rung, and whether execution may start. */
export function describeReadiness(state: ReadinessState): {
  label: string;
  canStartTask: boolean;
} {
  switch (state) {
    case "conversation_only":
      return {
        label: "Conversation only — no project attached",
        canStartTask: false,
      };
    case "selected":
      return {
        label: "Project selected — folder not available",
        canStartTask: false,
      };
    case "attached":
      return {
        label: "Project attached — runtime not configured",
        canStartTask: false,
      };
    case "runtime_configured":
      return {
        label: "Runtime configured — access not verified",
        canStartTask: false,
      };
    case "access_verified":
      return {
        label: "Access verified — execution not authorized (paused)",
        canStartTask: false,
      };
    case "execution_authorized":
      return { label: "Execution authorized", canStartTask: true };
    case "task_running":
      return { label: "Task running", canStartTask: true };
  }
}

/**
 * Split report text into lines for mixed-direction rendering: Latin-only
 * lines get `dir="ltr"`, everything else `dir="rtl"`, so English terms placed
 * on their own line by the coordinator stay readable inside Arabic prose.
 */
export function reportLines(
  text: string,
): { text: string; dir: "ltr" | "rtl" }[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trimEnd())
    .filter((line) => line.length > 0)
    .map((line) => ({
      text: line,
      dir: /[؀-ۿ]/.test(line) ? "rtl" : "ltr",
    }));
}

/** Dispatch state explanation, including what the owner must do for `uncertain`. */
export function describeDispatch(
  task: Pick<
    ProjectTask,
    "dispatch_state" | "dispatch_target" | "dispatch_event_id"
  >,
): string {
  switch (task.dispatch_state) {
    case "none":
      return "No notification pending.";
    case "pending":
      return `Notification to ${task.dispatch_target ?? "owner"} is pending.`;
    case "sent":
      return task.dispatch_event_id === "in-app"
        ? "Owner notified on this board."
        : `Notification delivered (event ${task.dispatch_event_id ?? "?"}).`;
    case "uncertain":
      return "Notification outcome unknown after a failed publish; check the channel, then confirm whether it arrived before allowing one more attempt.";
  }
}
