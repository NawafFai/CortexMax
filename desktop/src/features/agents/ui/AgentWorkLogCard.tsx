import * as React from "react";

import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { formatAgo, runtimeTeamLabel } from "@/features/agents/lib/teamRoster";
import {
  buildBucketBoundaries,
  formatTokens,
  formatUsd,
  readTokenField,
} from "@/features/usage/lib/usageWindow";
import {
  getAgentUsageSeries,
  getInvalidTurnsByAgent,
  getLastTurnByAgent,
} from "@/shared/api/tauriArchive";
import type { AgentUsageSeries } from "@/shared/api/tauriArchive";

/**
 * What each agent has actually been doing.
 *
 * Every fact here already existed — the model on the agent record, the turns
 * and tokens in the archive, the last error from the last run — but each lived
 * somewhere different, and an agent that had quietly stopped working looked
 * exactly like one with nothing to do. Reading a log file on disk was the only
 * way to tell them apart.
 *
 * Deliberately not shown: which files an agent touched. That is only
 * recoverable by parsing harness logs, whose format is not ours and changes
 * between versions; a column that is wrong after an upgrade is worse than one
 * that was never there.
 */

const STATUS_DOT: Record<string, string> = {
  running: "bg-emerald-500",
  ready: "bg-emerald-500",
  starting: "bg-amber-500",
  stopped: "bg-muted-foreground/40",
  error: "bg-red-500",
};

/** Seconds of history the log covers. A week spans a normal working rhythm. */
const WINDOW_DAYS = 7;

export function AgentWorkLogCard() {
  const agentsQuery = useManagedAgentsQuery();
  const [series, setSeries] = React.useState<AgentUsageSeries | null>(null);
  const [lastTurns, setLastTurns] = React.useState<Map<string, number>>(
    () => new Map(),
  );
  const [failedTurns, setFailedTurns] = React.useState<Map<string, number>>(
    () => new Map(),
  );

  React.useEffect(() => {
    void (async () => {
      try {
        const boundaries = buildBucketBoundaries(WINDOW_DAYS);
        const from = boundaries[0] ?? 0;
        const to = boundaries[boundaries.length - 1] ?? 0;
        const [usage, turns, failures] = await Promise.all([
          getAgentUsageSeries({ bucketBoundaries: boundaries }),
          getLastTurnByAgent(from, to),
          getInvalidTurnsByAgent(from, to),
        ]);
        setSeries(usage);
        setLastTurns(new Map(turns));
        setFailedTurns(new Map(failures));
      } catch {
        // Without the archive the card still lists agents and their state; it
        // simply cannot say what they spent.
        setSeries(null);
      }
    })();
  }, []);

  const usageByAgent = React.useMemo(() => {
    const map = new Map<string, AgentUsageSeries["agents"][number]>();
    for (const row of series?.agents ?? []) map.set(row.agentPubkey, row);
    return map;
  }, [series]);

  const now = Date.now() / 1000;
  const rows = React.useMemo(() => {
    const agents = [...(agentsQuery.data ?? [])];
    // Busiest first: the agents spending the most are the ones worth looking
    // at, and an idle agent at the top would bury them.
    agents.sort((left, right) => {
      const l =
        usageByAgent.get(left.pubkey)?.usage.estimatedCostUsd.value ?? 0;
      const r =
        usageByAgent.get(right.pubkey)?.usage.estimatedCostUsd.value ?? 0;
      return r - l || left.name.localeCompare(right.name);
    });
    return agents;
  }, [agentsQuery.data, usageByAgent]);

  if (rows.length === 0) return null;

  return (
    <SettingsOptionGroup
      description={`What each agent has been doing over the last ${WINDOW_DAYS} days: its model, when it last worked, what it spent, and its last error.`}
      title="Agent activity"
    >
      <ul className="divide-y divide-border/55">
        {rows.map((agent) => {
          const usage = usageByAgent.get(agent.pubkey);
          const turns = usage?.reportCount ?? 0;
          const spend = usage?.usage.estimatedCostUsd.value ?? null;
          const tokens = readTokenField(usage?.usage.inputTokens);
          const lastTurn = lastTurns.get(agent.pubkey) ?? null;
          const failed = failedTurns.get(agent.pubkey) ?? 0;

          return (
            <li className="space-y-1.5 px-4 py-3" key={agent.pubkey}>
              <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-xs">
                <span
                  aria-hidden="true"
                  className={`h-2 w-2 shrink-0 self-center rounded-full ${
                    STATUS_DOT[agent.status ?? ""] ?? "bg-muted-foreground/40"
                  }`}
                />
                <span className="min-w-0 truncate font-medium">
                  {agent.name}
                </span>
                <span className="shrink-0 text-muted-foreground">
                  {runtimeTeamLabel(agent.runtime)}
                </span>
                {/*
                  Failed turns are excluded from every spend figure, which is
                  right for money and misleading alone: an agent whose turns
                  keep failing looks exactly like an idle one until this says
                  otherwise.
                */}
                {failed > 0 ? (
                  <span
                    className="shrink-0 rounded bg-red-500/15 px-1.5 py-0.5 text-3xs font-medium tabular-nums text-red-600 dark:text-red-400"
                    title={`${failed} turn${failed === 1 ? "" : "s"} in the last ${WINDOW_DAYS} days reported metrics that could not be read — these are excluded from the spend figures`}
                  >
                    {failed} failed
                  </span>
                ) : null}
                <span className="ml-auto shrink-0 tabular-nums text-muted-foreground">
                  {turns} turn{turns === 1 ? "" : "s"}
                </span>
                <span className="w-16 shrink-0 text-right font-medium tabular-nums">
                  {formatUsd(spend)}
                </span>
              </div>

              <div className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5 pl-5 text-3xs text-muted-foreground">
                <span className="truncate">
                  {agent.model ?? "model not set"}
                </span>
                <span className="tabular-nums">
                  {lastTurn == null
                    ? "no turn recorded"
                    : `last turn ${formatAgo(now - lastTurn)}`}
                </span>
                <span className="tabular-nums">{formatTokens(tokens)} in</span>
              </div>

              {/*
                An agent's last error is the single most useful thing on this
                card and the hardest to find today — it lives in a log file on
                disk. A stopped agent that failed looks exactly like one that
                was stopped on purpose until this line says otherwise.
              */}
              {agent.lastError ? (
                <p className="ml-5 rounded bg-red-500/10 px-2 py-1 text-3xs leading-relaxed">
                  {agent.lastError}
                </p>
              ) : null}
            </li>
          );
        })}
      </ul>
    </SettingsOptionGroup>
  );
}
