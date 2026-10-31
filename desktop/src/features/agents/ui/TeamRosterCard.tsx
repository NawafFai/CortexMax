import * as React from "react";

import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  rollupTeams,
  teamShare,
  type TeamRow,
} from "@/features/agents/lib/teamRoster";
import {
  ProviderLimitBars,
  useProviderLimits,
} from "@/features/usage/ui/ProviderLimits";

/**
 * The team, split by the runtime each agent runs on.
 *
 * A thirty-agent team is really several teams, and they matter separately
 * because each runtime has its own provider limit. "How many are on Codex" is
 * the question behind "can I keep working once Claude is spent", and it could
 * previously only be answered by opening every agent in turn.
 *
 * Each team's limit sits beside its headcount, because the two are only useful
 * together: twenty agents on a runtime with no room left is a different
 * situation from twenty on one that is barely touched.
 */

/** Distinct colours per team, so a row is recognizable at a glance. */
const TEAM_BAR: Record<string, string> = {
  Claude: "bg-orange-500",
  Codex: "bg-sky-500",
  OpenCode: "bg-violet-500",
};
const FALLBACK_BAR = "bg-muted-foreground/50";

function TeamLine({ row, total }: { row: TeamRow; total: number }) {
  const share = Math.round(teamShare(row, total) * 100);
  return (
    <li className="space-y-1.5 px-4 py-3">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-xs">
        <span className="font-medium">{row.label}</span>
        <span className="text-muted-foreground">
          {row.running} of {row.total} running
        </span>
        <span className="ml-auto shrink-0 font-medium tabular-nums">
          {row.total}
        </span>
      </div>
      <div
        aria-label={`${row.label} share of the team`}
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={share}
        className="h-2 w-full overflow-hidden rounded-full bg-muted"
        role="progressbar"
      >
        {/*
          The filled part is the whole team on this runtime; the darker inner
          part is how many are actually up. One bar answers both "how much of
          my team is here" and "how much of it is working".
        */}
        <div
          className={`h-full rounded-full ${TEAM_BAR[row.label] ?? FALLBACK_BAR}`}
          style={{ width: `${share}%` }}
        >
          <div
            className="h-full rounded-full bg-foreground/25"
            style={{
              width: `${row.total > 0 ? (row.running / row.total) * 100 : 0}%`,
            }}
          />
        </div>
      </div>
    </li>
  );
}

export function TeamRosterCard() {
  const agentsQuery = useManagedAgentsQuery();
  const { groups } = useProviderLimits();

  const agents = agentsQuery.data ?? [];
  const rows = React.useMemo(
    () =>
      rollupTeams(
        agents.map((agent) => ({
          runtime: agent.runtime,
          status: agent.status,
        })),
      ),
    [agents],
  );

  if (rows.length === 0) return null;
  const total = agents.length;
  const running = rows.reduce((sum, row) => sum + row.running, 0);

  return (
    <SettingsOptionGroup
      description="Your agents grouped by the runtime they run on, with each runtime's own plan limit."
      title="Teams"
    >
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 px-4 py-3">
        <p className="text-xs font-medium">
          {total} agent{total === 1 ? "" : "s"} across {rows.length} runtime
          {rows.length === 1 ? "" : "s"}
        </p>
        <p className="text-3xs tabular-nums text-muted-foreground">
          {running} running now
        </p>
      </div>

      <ul className="divide-y divide-border/55">
        {rows.map((row) => (
          <TeamLine key={row.runtime || "none"} row={row} total={total} />
        ))}
      </ul>

      {groups && groups.length > 0 ? (
        <div className="px-4 py-3.5">
          <h4 className="mb-2.5 text-xs font-medium text-muted-foreground">
            Room left on each runtime
          </h4>
          <ProviderLimitBars groups={groups} />
        </div>
      ) : null}
    </SettingsOptionGroup>
  );
}
