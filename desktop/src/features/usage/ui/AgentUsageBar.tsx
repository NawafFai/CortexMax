import * as React from "react";

import {
  getAgentUsageRollup,
  onAgentMetricsChanged,
} from "@/shared/api/tauriArchive";
import type { ChannelUsage } from "@/shared/api/tauriArchive";
import {
  budgetFraction,
  budgetZone,
  buildBucketBoundaries,
  formatTokens,
  formatUsd,
} from "@/features/usage/lib/usageWindow";
import { readDailyBudget } from "@/features/usage/lib/budgetStorage";

/**
 * What one agent has spent today, for its row in the members sidebar.
 *
 * The channel bar answers "what did this project cost"; scanning a team to
 * find *which member* is doing the spending needs the figure next to the
 * name. Same window and same green/amber/red scale as the channel bar above
 * it, so the two describe the same day on the same terms.
 *
 * The zone is measured against a per-agent share of the daily budget rather
 * than the whole budget: one agent at 90% of the team's entire day is already
 * an emergency, and a bar that only reddens there would never warn in time.
 */

/**
 * Share of the daily budget a single agent is expected to stay under. A team
 * is usually several agents, so one taking a fifth of the day's budget alone
 * is the point worth colouring.
 */
const AGENT_BUDGET_SHARE = 0.2;

function readAgentBudget(): number {
  return readDailyBudget() * AGENT_BUDGET_SHARE;
}

const ZONE_BAR: Record<string, string> = {
  ok: "bg-emerald-500",
  warn: "bg-amber-500",
  over: "bg-red-500",
};

export function AgentUsageBar({ agentPubkey }: { agentPubkey: string }) {
  const [usage, setUsage] = React.useState<ChannelUsage | null>(null);
  const budget = React.useMemo(readAgentBudget, []);

  const load = React.useCallback(async () => {
    const boundaries = buildBucketBoundaries(1);
    try {
      setUsage(
        await getAgentUsageRollup(
          agentPubkey,
          boundaries[0],
          boundaries[1] ?? 0,
        ),
      );
    } catch {
      setUsage(null);
    }
  }, [agentPubkey]);

  React.useEffect(() => {
    void load();
  }, [load]);

  React.useEffect(() => {
    const unsubscribe = onAgentMetricsChanged(() => {
      void load();
    });
    return () => {
      unsubscribe();
    };
  }, [load]);

  // An agent that has not worked today shows nothing: an empty track would
  // read as a measurement rather than an absence.
  if (!usage || usage.turns === 0) return null;

  const priced = usage.costUsd != null;
  const zone = budgetZone(usage.costUsd, budget) ?? "ok";
  const percent = Math.round(budgetFraction(usage.costUsd, budget) * 100);
  const floor = usage.turnsWithoutCost > 0;

  return (
    <span
      className="inline-flex items-center gap-1.5"
      data-testid={`sidebar-member-usage-${agentPubkey}`}
      title={
        priced
          ? `${formatUsd(usage.costUsd)} today over ${usage.turns} turns in ${usage.agents} channel(s)${
              floor ? ", plus turns whose harness reported no price" : ""
            }`
          : `${usage.turns} turns today; this harness reports no price`
      }
    >
      {priced ? (
        <span
          aria-label="Agent spend today"
          aria-valuemax={100}
          aria-valuemin={0}
          aria-valuenow={percent}
          className="h-1 w-10 overflow-hidden rounded-full bg-muted"
          role="progressbar"
        >
          <span
            className={`block h-full rounded-full ${ZONE_BAR[zone]}`}
            style={{ width: `${percent}%` }}
          />
        </span>
      ) : null}
      <span className="text-3xs tabular-nums text-muted-foreground">
        {priced
          ? `${floor ? "≥" : ""}${formatUsd(usage.costUsd)}`
          : `${formatTokens(usage.inputTokens)} in`}
      </span>
    </span>
  );
}
