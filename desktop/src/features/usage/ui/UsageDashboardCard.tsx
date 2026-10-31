import * as React from "react";

import {
  getAgentUsageSeries,
  onAgentMetricsChanged,
} from "@/shared/api/tauriArchive";
import type { AgentUsageSeries } from "@/shared/api/tauriArchive";
import type {
  RuntimeBudgets,
  RuntimeUsageRow,
} from "@/features/usage/lib/usageWindow";
import {
  BUDGET_OVER_AT,
  BUDGET_WARN_AT,
  budgetFraction,
  budgetZone,
  buildBucketBoundaries,
  formatTokens,
  formatUsd,
  coverageNote,
  readTokenField,
  rollupByRuntime,
  runtimeBudget,
  USAGE_WINDOWS,
  type UsageWindowDays,
} from "@/features/usage/lib/usageWindow";

import {
  readDailyBudget,
  readRuntimeBudgets,
  writeDailyBudget,
  writeRuntimeBudgets,
} from "@/features/usage/lib/budgetStorage";
import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";
import { truncateNpub } from "@/shared/lib/pubkey";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { BreakResumeControl } from "@/features/usage/ui/BreakResumeControl";
import { CostProfileControl } from "@/features/usage/ui/CostProfileControl";
import { ProviderLimitsSection } from "@/features/usage/ui/ProviderLimits";
import { SiblingConflictBanner } from "@/features/usage/ui/SiblingConflictBanner";
import { BudgetGovernorControl } from "@/features/usage/ui/BudgetGovernorControl";
import { ModelComparisonCard } from "@/features/usage/ui/ModelComparisonCard";
import { ModelFailoverControl } from "@/features/usage/ui/ModelFailoverControl";
import { SessionRecyclerControl } from "@/features/usage/ui/SessionRecyclerControl";
import { WaveSchedulerControl } from "@/features/usage/ui/WaveSchedulerControl";

/**
 * Owner-facing spend dashboard over the locally archived NIP-AM series.
 *
 * Every managed-agent turn already reports its own token counts and, when the
 * harness knows its pricing, a cost. Nothing surfaced them, so a team could
 * exhaust a subscription limit with no warning. This card answers: what did
 * today cost against the budget I set, which agent is spending it, and how
 * does today compare with the days before.
 *
 * The cost figures are what each harness reports, not a charge: on a
 * subscription the meaningful limit is the provider's own session cap, and
 * these dollars are the equivalent API value. The card says so rather than
 * letting the number read as a bill.
 */

/** Agents listed before the table folds into a "N more" line. */
const AGENT_ROWS = 12;

const ZONE_BAR: Record<string, string> = {
  ok: "bg-emerald-500",
  warn: "bg-amber-500",
  over: "bg-red-500",
};

const ZONE_TEXT: Record<string, string> = {
  ok: "text-emerald-600 dark:text-emerald-400",
  warn: "text-amber-600 dark:text-amber-400",
  over: "text-red-600 dark:text-red-400",
};

const ZONE_LABEL: Record<string, string> = {
  ok: "Within budget",
  warn: "Approaching budget",
  over: "Over budget",
};

/** The green/amber/red track, with the threshold ticks the zones use. */
function BudgetBar({
  spend,
  budget,
}: {
  spend: number | null;
  budget: number;
}) {
  const zone = budgetZone(spend, budget) ?? "ok";
  const fraction = budgetFraction(spend, budget);
  const percent = Math.round(fraction * 100);
  return (
    <div className="space-y-1.5">
      <div
        aria-label="Spend against daily budget"
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={percent}
        className="relative h-3 w-full overflow-hidden rounded-full bg-muted"
        role="progressbar"
      >
        <div
          className={`h-full rounded-full transition-[width] duration-500 ${ZONE_BAR[zone]}`}
          style={{ width: `${percent}%` }}
        />
        {[BUDGET_WARN_AT, BUDGET_OVER_AT].map((threshold) => (
          <span
            aria-hidden="true"
            className="absolute top-0 h-full w-px bg-background/70"
            key={threshold}
            style={{ left: `${threshold * 100}%` }}
          />
        ))}
      </div>
      <div className="flex items-baseline justify-between text-xs">
        <span className={`font-medium ${ZONE_TEXT[zone]}`}>
          {ZONE_LABEL[zone]} · {percent}%
        </span>
        <span className="text-muted-foreground">
          {formatUsd(spend)} of {formatUsd(budget)} today
        </span>
      </div>
    </div>
  );
}

/** Format one bucket's start as a short day label. */
function dayLabel(start: number): string {
  return new Date(start * 1000).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
  });
}

/**
 * The daily history, with the budget drawn across it as a reference line.
 *
 * A column chart alone answers "which day was biggest"; the owner's actual
 * question is "which days went over what I allow", and that is only readable
 * when the budget sits on the same axis as the bars. The line is omitted when
 * the budget is above every bar, where it would pin to the ceiling and say
 * nothing.
 *
 * Bars and labels are two rows sharing one gap, rather than one column each:
 * the bar row alone is the value axis, so the reference line can be positioned
 * against it without the label row skewing the percentage.
 */
function HistoryChart({
  buckets,
  budget,
  tallest,
  windowDays,
}: {
  buckets: AgentUsageSeries["buckets"];
  budget: number;
  tallest: number;
  windowDays: UsageWindowDays;
}) {
  // Roughly six labels whatever the window, and always the most recent day.
  const stride = Math.max(1, Math.ceil(buckets.length / 6));
  const gap = windowDays > 7 ? "gap-px" : "gap-1";
  const peak = buckets.reduce<{ value: number; label: string } | null>(
    (best, bucket) => {
      const value = bucket.usage.estimatedCostUsd.value;
      if (value == null || (best != null && value <= best.value)) return best;
      return { value, label: dayLabel(bucket.start) };
    },
    null,
  );
  const budgetLineAt = tallest > 0 ? (budget / tallest) * 100 : 0;
  const showBudgetLine = budgetLineAt > 0 && budgetLineAt < 97;

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 text-3xs text-muted-foreground">
        <span>
          {peak
            ? `Peak ${formatUsd(peak.value)} on ${peak.label}`
            : "No priced spend in this window"}
        </span>
        {showBudgetLine ? (
          <span className="tabular-nums">
            Dashed line · {formatUsd(budget)} daily budget
          </span>
        ) : null}
      </div>

      <div className={`relative flex h-28 items-end ${gap}`}>
        {showBudgetLine ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 z-10 border-t border-dashed border-foreground/40"
            style={{ bottom: `${budgetLineAt}%` }}
          />
        ) : null}
        {buckets.map((bucket) => {
          const spend = bucket.usage.estimatedCostUsd.value;
          const zone = budgetZone(spend, budget) ?? "ok";
          const height =
            tallest > 0 && spend != null
              ? Math.max(2, Math.round((spend / tallest) * 100))
              : 2;
          return (
            <div
              className={`min-w-0 flex-1 rounded-t-sm ${ZONE_BAR[zone]} ${
                spend == null ? "opacity-40" : ""
              }`}
              key={bucket.start}
              style={{ height: `${height}%` }}
              title={`${dayLabel(bucket.start)}: ${
                spend == null ? "no usage reported" : formatUsd(spend)
              }`}
            />
          );
        })}
      </div>

      {/*
        Labels keep one slot per bar so they stay under the day they name, but
        a slot in a 30-day window is narrower than any date. Only sampled slots
        carry text, so the text is allowed to overflow into its empty
        neighbours rather than truncate to "Aug …".
      */}
      <div aria-hidden="true" className={`flex ${gap}`}>
        {buckets.map((bucket, index) => (
          <span
            className="min-w-0 flex-1 overflow-visible whitespace-nowrap text-center text-3xs leading-none text-muted-foreground"
            key={bucket.start}
          >
            {index === buckets.length - 1 ||
            (buckets.length - 1 - index) % stride === 0
              ? dayLabel(bucket.start)
              : ""}
          </span>
        ))}
      </div>
    </div>
  );
}

/**
 * One runtime's line: its own budget, its own bar, and an explicit note when
 * the harness does not report enough to draw one.
 */
function RuntimeRow({
  row,
  budget,
  onBudgetChange,
}: {
  row: RuntimeUsageRow;
  budget: number | null;
  onBudgetChange: (value: number) => void;
}) {
  const priced = row.coverage === "full";
  const zone = priced ? (budgetZone(row.costUsd, budget) ?? "ok") : null;
  // A zero limit is "no limit", not "every spend is infinitely over": an
  // empty input parses to 0 and must not paint the row red.
  const hasLimit = priced && budget != null && budget > 0;
  const percent = hasLimit
    ? Math.round(budgetFraction(row.costUsd, budget) * 100)
    : 0;
  const inputId = `runtime-budget-${row.harness}`;
  return (
    <li className="space-y-2 px-4 py-3">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-xs">
        <span className="font-medium">{row.label}</span>
        <span className="text-muted-foreground">{row.turns} turns</span>
        <span className="tabular-nums text-muted-foreground">
          {formatTokens(row.inputTokens)} in
        </span>
        <span className="ml-auto flex items-baseline gap-2">
          {hasLimit ? (
            <span
              className={`text-3xs font-medium tabular-nums ${ZONE_TEXT[zone ?? "ok"]}`}
            >
              {percent}%
            </span>
          ) : null}
          <span className="font-medium tabular-nums">
            {priced ? formatUsd(row.costUsd) : "not priced"}
          </span>
        </span>
      </div>
      {priced ? (
        <>
          {/*
            A track with no limit behind it is decoration: it can only ever
            render empty, which reads as "nothing spent" rather than "nothing
            to measure against". The input below is the affordance instead.
          */}
          {hasLimit ? (
            <div
              aria-label={`${row.label} spend today against its daily limit`}
              aria-valuemax={100}
              aria-valuemin={0}
              aria-valuenow={percent}
              className="h-2 w-full overflow-hidden rounded-full bg-muted"
              role="progressbar"
            >
              <div
                className={`h-full rounded-full transition-[width] duration-500 ${
                  ZONE_BAR[zone ?? "ok"]
                }`}
                style={{ width: `${percent}%` }}
              />
            </div>
          ) : null}
          <div className="flex items-center justify-between gap-3">
            <label
              className="min-w-0 text-3xs text-muted-foreground"
              htmlFor={inputId}
            >
              {hasLimit
                ? "Daily limit (USD)"
                : `Daily limit (USD) — set one to track ${row.label} against it`}
            </label>
            <input
              className="h-7 w-24 shrink-0 rounded border bg-background px-2 text-xs tabular-nums"
              id={inputId}
              inputMode="decimal"
              min={0}
              onChange={(event) => {
                const next = Number(event.target.value);
                if (!Number.isFinite(next) || next < 0) return;
                onBudgetChange(next);
              }}
              placeholder="not set"
              step={5}
              type="number"
              value={budget ?? ""}
            />
          </div>
        </>
      ) : (
        <p className="text-3xs leading-relaxed text-muted-foreground">
          {coverageNote(row.coverage)}.
          {row.coverage === "tokens-only"
            ? " Watch its token count instead; a limit here would be guesswork."
            : null}
        </p>
      )}
    </li>
  );
}

export function UsageDashboardCard({
  agentNames,
}: {
  /**
   * Pubkey to display name. Optional: when the caller supplies nothing the
   * card resolves names from the managed-agent store itself, so the table
   * reads as people rather than keys wherever it is mounted.
   */
  agentNames?: Record<string, string>;
}) {
  const managedAgentsQuery = useManagedAgentsQuery();
  const resolvedNames = React.useMemo(() => {
    if (agentNames) return agentNames;
    const map: Record<string, string> = {};
    for (const agent of managedAgentsQuery.data ?? []) {
      if (agent.pubkey && agent.name) map[agent.pubkey] = agent.name;
    }
    return map;
  }, [agentNames, managedAgentsQuery.data]);
  const [windowDays, setWindowDays] = React.useState<UsageWindowDays>(7);
  const [budget, setBudget] = React.useState<number>(readDailyBudget);
  const [runtimeBudgets, setRuntimeBudgets] =
    React.useState<RuntimeBudgets>(readRuntimeBudgets);
  const [series, setSeries] = React.useState<AgentUsageSeries | null>(null);
  const [todayAgents, setTodayAgents] = React.useState<
    AgentUsageSeries["agents"]
  >([]);
  const [error, setError] = React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(true);

  const load = React.useCallback(async () => {
    setLoading(true);
    try {
      // Two windows, because they answer different questions: the chart and
      // the agent table describe the selected window, while a runtime limit is
      // a *daily* allowance — the same figure the sidebar strip shows. Reading
      // the window total against a daily limit would report a team as 4x over
      // simply for having run all week.
      const [windowResult, todayResult] = await Promise.all([
        getAgentUsageSeries({
          bucketBoundaries: buildBucketBoundaries(windowDays),
        }),
        getAgentUsageSeries({ bucketBoundaries: buildBucketBoundaries(1) }),
      ]);
      setSeries(windowResult);
      setTodayAgents(todayResult.agents);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, [windowDays]);

  React.useEffect(() => {
    void load();
  }, [load]);

  // Refresh when the archive indexes new turns, so the bar tracks a running
  // team instead of going stale until the panel is reopened.
  React.useEffect(() => {
    const unsubscribe = onAgentMetricsChanged(() => {
      void load();
    });
    return () => {
      unsubscribe();
    };
  }, [load]);

  const buckets = series?.buckets ?? [];
  const todayBucket = buckets.at(-1) ?? null;
  const todaySpend = todayBucket?.usage.estimatedCostUsd.value ?? null;
  const windowSpend = buckets.reduce<number | null>((total, bucket) => {
    const value = bucket.usage.estimatedCostUsd.value;
    if (value == null) return total;
    return (total ?? 0) + value;
  }, null);
  const tallest = buckets.reduce(
    (max, bucket) => Math.max(max, bucket.usage.estimatedCostUsd.value ?? 0),
    0,
  );

  const agents = React.useMemo(() => {
    const rows = [...(series?.agents ?? [])];
    rows.sort((left, right) => {
      const l = left.usage.estimatedCostUsd.value ?? 0;
      const r = right.usage.estimatedCostUsd.value ?? 0;
      if (r !== l) return r - l;
      return (
        (readTokenField(right.usage.inputTokens) ?? 0) -
        (readTokenField(left.usage.inputTokens) ?? 0)
      );
    });
    return rows;
  }, [series]);

  const runtimes = React.useMemo(
    () => rollupByRuntime(todayAgents),
    [todayAgents],
  );

  const incomplete =
    series?.coverage.hasUnknownUsage ||
    (series?.coverage.invalidReportCount ?? 0) > 0;

  return (
    <SettingsOptionGroup
      description="What your agents are spending, from the usage each harness reports for its own turns."
      title="Usage"
    >
      {/*
        Each block below is a direct child of the section card, which supplies
        the dividers between them; the horizontal padding is this card's job,
        which is why every block carries `px-4` rather than sharing one wrapper.
      */}
      <SiblingConflictBanner />

      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-4 py-3">
        <div className="min-w-0">
          <p className="text-xs font-medium">Run control</p>
          <p className="text-3xs text-muted-foreground">
            Break stops every running agent; Start brings back exactly the ones
            it stopped.
          </p>
        </div>
        <BreakResumeControl />
      </div>

      <ProviderLimitsSection />

      <BudgetGovernorControl />

      <SessionRecyclerControl />

      <WaveSchedulerControl />

      <ModelFailoverControl />
      <CostProfileControl />

      {series && !series.collectionEnabled ? (
        <p className="bg-amber-500/10 px-4 py-3 text-xs">
          Usage collection is turned off, so these totals stop at the last
          archived turn. Enable agent metrics in Local archive to resume.
        </p>
      ) : null}

      {error ? (
        <p className="bg-red-500/10 px-4 py-3 text-xs">
          Could not read usage: {error}
        </p>
      ) : null}

      <div className="space-y-2.5 px-4 py-3.5">
        <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
          <label className="text-xs font-medium" htmlFor="usage-daily-budget">
            Daily budget (USD)
          </label>
          <input
            className="h-8 w-28 shrink-0 rounded border bg-background px-2 text-sm tabular-nums"
            id="usage-daily-budget"
            inputMode="decimal"
            min={0}
            onChange={(event) => {
              const next = Number(event.target.value);
              if (!Number.isFinite(next) || next < 0) return;
              setBudget(next);
              writeDailyBudget(next);
            }}
            step={5}
            type="number"
            value={budget}
          />
        </div>
        <BudgetBar budget={budget} spend={todaySpend} />
      </div>

      <div className="space-y-3 px-4 py-3.5">
        <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
          <div className="flex items-center gap-1.5">
            {USAGE_WINDOWS.map((days) => (
              <button
                className={`rounded border px-2.5 py-1 text-xs transition-colors ${
                  windowDays === days
                    ? "border-foreground/30 bg-muted font-medium"
                    : "border-transparent text-muted-foreground hover:bg-muted/60"
                }`}
                key={days}
                onClick={() => setWindowDays(days)}
                type="button"
              >
                {days} days
              </button>
            ))}
          </div>
          <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
            {formatUsd(windowSpend)} over {windowDays} days
          </span>
        </div>

        {loading && !series ? (
          <p className="text-xs text-muted-foreground">Reading usage…</p>
        ) : null}

        {buckets.length > 0 ? (
          <HistoryChart
            buckets={buckets}
            budget={budget}
            tallest={tallest}
            windowDays={windowDays}
          />
        ) : null}
      </div>

      {runtimes.length > 0 ? (
        <div className="px-4 py-3.5">
          <h4 className="text-xs font-medium text-muted-foreground">
            By runtime · today
          </h4>
          {/*
            Full-bleed so each row's divider spans the card, the way the rest
            of Settings reads, instead of drawing a second border inside one.
          */}
          <ul className="-mx-4 mt-2.5 divide-y divide-border/55 border-t border-border/55">
            {runtimes.map((row) => (
              <RuntimeRow
                budget={runtimeBudget(runtimeBudgets, row.harness)}
                key={row.harness}
                onBudgetChange={(value) => {
                  const next = { ...runtimeBudgets, [row.harness]: value };
                  setRuntimeBudgets(next);
                  writeRuntimeBudgets(next);
                }}
                row={row}
              />
            ))}
          </ul>
        </div>
      ) : null}

      <ModelComparisonCard series={series} windowDays={windowDays} />

      {agents.length > 0 ? (
        <div className="px-4 py-3.5">
          <h4 className="text-xs font-medium text-muted-foreground">
            By agent · {windowDays} days
          </h4>
          <ul className="-mx-4 mt-2.5 divide-y divide-border/55 border-t border-border/55">
            {agents.slice(0, AGENT_ROWS).map((agent) => {
              const spend = agent.usage.estimatedCostUsd.value;
              const zone = budgetZone(spend, budget) ?? "ok";
              return (
                <li
                  className="flex items-center gap-3 px-4 py-2 text-xs"
                  key={agent.agentPubkey}
                >
                  <span
                    aria-hidden="true"
                    className={`h-2 w-2 shrink-0 rounded-full ${ZONE_BAR[zone]}`}
                  />
                  <span className="min-w-0 flex-1 truncate">
                    {resolvedNames[agent.agentPubkey] ??
                      truncateNpub(agent.agentPubkey)}
                  </span>
                  <span className="shrink-0 tabular-nums text-muted-foreground">
                    {formatTokens(readTokenField(agent.usage.inputTokens))} in
                  </span>
                  <span className="w-16 shrink-0 text-right tabular-nums text-muted-foreground">
                    {agent.reportCount} turns
                  </span>
                  <span className="w-16 shrink-0 text-right font-medium tabular-nums">
                    {formatUsd(spend)}
                  </span>
                </li>
              );
            })}
          </ul>
          {agents.length > AGENT_ROWS ? (
            <p className="mt-2 text-3xs text-muted-foreground">
              {agents.length - AGENT_ROWS} more{" "}
              {agents.length - AGENT_ROWS === 1 ? "agent" : "agents"} spent less
              than{" "}
              {formatUsd(
                agents[AGENT_ROWS - 1]?.usage.estimatedCostUsd.value ?? null,
              )}
              .
            </p>
          ) : null}
        </div>
      ) : null}

      <p className="px-4 py-3 text-2xs leading-relaxed text-muted-foreground">
        These dollars are what each harness reports for its own turns. On a
        subscription they are the equivalent API value, not a charge — the limit
        you actually hit is the provider&apos;s session cap.
        {incomplete
          ? " Some turns reported no usage, so the totals are a floor, not a ceiling."
          : null}
      </p>
    </SettingsOptionGroup>
  );
}
