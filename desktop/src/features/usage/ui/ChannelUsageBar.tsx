import * as React from "react";

import {
  getChannelUsage,
  onAgentMetricsChanged,
} from "@/shared/api/tauriArchive";
import type { ChannelUsage } from "@/shared/api/tauriArchive";
import {
  budgetFraction,
  budgetZone,
  buildBucketBoundaries,
  channelBudget,
  formatTokens,
  formatUsd,
} from "@/features/usage/lib/usageWindow";
import {
  readChannelBudgets,
  readDailyBudget,
} from "@/features/usage/lib/budgetStorage";

/**
 * Spend this channel has cost today, on the same green/amber/red scale the
 * usage dashboard uses.
 *
 * A team runs inside a channel, so the channel is the unit an owner actually
 * budgets: "what has this project cost me" is answerable here and nowhere
 * else in the app. The bar reads the channel's own rollup rather than the
 * dashboard's agent series, and scales it against this project's own budget,
 * falling back to the account-wide daily figure where none is set.
 */

const ZONE_BAR: Record<string, string> = {
  ok: "bg-emerald-500",
  warn: "bg-amber-500",
  over: "bg-red-500",
};

export function ChannelUsageBar({
  channelId,
  className,
}: {
  channelId: string;
  className?: string;
}) {
  const [usage, setUsage] = React.useState<ChannelUsage | null>(null);
  const budget = React.useMemo(
    () => channelBudget(readChannelBudgets(), channelId, readDailyBudget()),
    [channelId],
  );

  const load = React.useCallback(async () => {
    // The final pair of daily boundaries is today: [today 00:00, tomorrow 00:00).
    const boundaries = buildBucketBoundaries(1);
    try {
      setUsage(
        await getChannelUsage(channelId, boundaries[0], boundaries[1] ?? 0),
      );
    } catch {
      // A channel with no archived turns is the common case on a fresh
      // channel; leaving the bar hidden is the right outcome either way.
      setUsage(null);
    }
  }, [channelId]);

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

  // Nothing spent here today: no bar, rather than an empty track that reads
  // as a measurement.
  if (!usage || usage.turns === 0) return null;

  const zone = budgetZone(usage.costUsd, budget) ?? "ok";
  const percent = Math.round(budgetFraction(usage.costUsd, budget) * 100);
  const priced = usage.costUsd != null;
  const floor = usage.turnsWithoutCost > 0;

  return (
    <div
      className={["flex items-center gap-2", className]
        .filter(Boolean)
        .join(" ")}
      title={
        priced
          ? `${formatUsd(usage.costUsd)} today across ${usage.turns} turns by ${usage.agents} agent(s)${
              floor ? ", plus turns whose harness reported no price" : ""
            }`
          : `${usage.turns} turns today; this channel's harnesses report no price`
      }
    >
      {priced ? (
        <div
          aria-label="Channel spend today against this project's budget"
          aria-valuemax={100}
          aria-valuemin={0}
          aria-valuenow={percent}
          className="h-1.5 w-16 overflow-hidden rounded-full bg-muted"
          role="progressbar"
        >
          <div
            className={`h-full rounded-full ${ZONE_BAR[zone]}`}
            style={{ width: `${percent}%` }}
          />
        </div>
      ) : null}
      <span className="whitespace-nowrap text-2xs tabular-nums text-muted-foreground">
        {priced
          ? `${floor ? "≥" : ""}${formatUsd(usage.costUsd)} today`
          : `${formatTokens(usage.inputTokens)} in today`}
      </span>
    </div>
  );
}
