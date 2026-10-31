/**
 * Group the team by the runtime each agent runs on.
 *
 * A thirty-agent team is really several teams — a Claude team, a Codex team —
 * and they matter separately because each runtime has its own provider limit.
 * "How many are on Codex" is the question behind "can I keep working once
 * Claude is spent", and until now it could only be answered by opening every
 * agent in turn.
 */

/** Harness ids shortened to the names the runtimes are actually called. */
const RUNTIME_LABELS: Record<string, string> = {
  claude: "Claude",
  "claude-agent-acp": "Claude",
  "claude-code-acp": "Claude",
  codex: "Codex",
  "codex-acp": "Codex",
  opencode: "OpenCode",
  kimi: "Kimi",
  goose: "Goose",
  "buzz-agent": "Buzz Agent",
};

/** The display name for a runtime id, unknown ids passing through unchanged. */
export function runtimeTeamLabel(runtime: string | null): string {
  if (!runtime) return "No runtime set";
  return RUNTIME_LABELS[runtime] ?? runtime;
}

/** Just enough of an agent to group it. */
export type RosterAgent = {
  runtime: string | null;
  status?: string | null;
};

/** One runtime's team. */
export type TeamRow = {
  /** Harness id, or "" for agents with no runtime set. */
  runtime: string;
  label: string;
  total: number;
  /** How many of them currently have a runtime process. */
  running: number;
};

/** Statuses that mean the agent is actually up. */
const RUNNING_STATUSES = new Set(["running", "starting", "ready"]);

/**
 * Count each runtime's team, largest first.
 *
 * Agents with no runtime set are kept as their own row rather than dropped:
 * they are real agents the owner has, and a roster that silently omits them
 * would not add up to the team they know they have.
 */
export function rollupTeams(agents: RosterAgent[]): TeamRow[] {
  const byRuntime = new Map<string, TeamRow>();

  for (const agent of agents) {
    const runtime = agent.runtime ?? "";
    let row = byRuntime.get(runtime);
    if (!row) {
      row = {
        runtime,
        label: runtimeTeamLabel(agent.runtime),
        total: 0,
        running: 0,
      };
      byRuntime.set(runtime, row);
    }
    row.total += 1;
    if (agent.status && RUNNING_STATUSES.has(agent.status)) {
      row.running += 1;
    }
  }

  return [...byRuntime.values()].sort(
    (left, right) =>
      right.total - left.total || left.label.localeCompare(right.label),
  );
}

/** Share of the whole team this row represents, 0-1. */
export function teamShare(row: TeamRow, agents: number): number {
  return agents > 0 ? row.total / agents : 0;
}

/**
 * How long ago something happened, for a work log.
 *
 * The unit follows the age, because the precision that matters changes with
 * it: an agent that worked four days ago is not usefully described in minutes.
 * A future timestamp — a clock correction, a restored snapshot — reads as
 * "just now" rather than as a negative age.
 */
export function formatAgo(secondsAgo: number): string {
  if (!Number.isFinite(secondsAgo)) return "unknown";
  if (secondsAgo < 60) return "just now";

  const minutes = Math.floor(secondsAgo / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}
