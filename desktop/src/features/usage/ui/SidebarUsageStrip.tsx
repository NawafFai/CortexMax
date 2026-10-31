import * as React from "react";

import {
  getAgentUsageSeries,
  onAgentMetricsChanged,
} from "@/shared/api/tauriArchive";
import type { AgentUsageSeries } from "@/shared/api/tauriArchive";
import {
  budgetFraction,
  budgetZone,
  buildBucketBoundaries,
  coverageNote,
  formatTokens,
  formatUsd,
  limitZone,
  remainingPercent,
  rollupByRuntime,
  runtimeBudget,
  type RuntimeUsageRow,
} from "@/features/usage/lib/usageWindow";
import {
  readDailyBudget,
  readRuntimeBudgets,
} from "@/features/usage/lib/budgetStorage";
import {
  ProviderLimitBars,
  useProviderLimits,
} from "@/features/usage/ui/ProviderLimits";
import type { LimitGroup, LimitWindow } from "@/shared/api/tauri";

/**
 * Always-visible limit strip at the top of the sidebar.
 *
 * The dashboard in Settings answers questions an owner goes looking for. This
 * answers the one they need without looking: how close is the team to the
 * ceiling that will stop it. A limit is only useful while there is still time
 * to act on it, which means it has to be on screen during the run, not two
 * clicks away.
 *
 * Plan limits lead, because they are the real constraint — the provider's own
 * 5-hour and weekly windows, reported by each runtime for itself. The
 * collapsed strip shows the fullest of them, since that is the one that will
 * bind first.
 *
 * Spend follows, measured against the owner's own daily budget. Those dollars
 * are what each harness says its turns were worth; on a subscription they are
 * API-equivalent value rather than a bill, which is why they sit below the
 * limits rather than above them. A runtime with no budget set shows its figure
 * without a bar, since a bar with no scale is decoration, and a runtime whose
 * harness reports no price shows tokens instead.
 */

const COLLAPSED_STORAGE_KEY = "cortex-max.usage-strip-collapsed";

function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return fallback;
    const parsed: unknown = JSON.parse(raw);
    return (parsed as T) ?? fallback;
  } catch {
    return fallback;
  }
}

const ZONE_BAR: Record<string, string> = {
  ok: "bg-emerald-500",
  warn: "bg-amber-500",
  over: "bg-red-500",
};

function RuntimeLine({
  row,
  limit,
}: {
  row: RuntimeUsageRow;
  limit: number | null;
}) {
  const priced = row.coverage === "full";
  const zone = budgetZone(row.costUsd, limit) ?? "ok";
  const percent =
    priced && limit != null
      ? Math.round(budgetFraction(row.costUsd, limit) * 100)
      : null;

  return (
    <div
      className="space-y-1"
      title={
        priced
          ? `${row.label}: ${formatUsd(row.costUsd)} today over ${row.turns} turns${
              limit == null ? " — no limit set in Settings > Usage" : ""
            }`
          : `${row.label}: ${row.turns} turns today — ${coverageNote(row.coverage)}`
      }
    >
      <div className="flex items-baseline justify-between gap-2 text-2xs">
        <span className="truncate text-muted-foreground">{row.label}</span>
        <span className="shrink-0 tabular-nums text-muted-foreground">
          {priced
            ? percent == null
              ? formatUsd(row.costUsd)
              : `${percent}% · ${formatUsd(row.costUsd)}`
            : `${formatTokens(row.inputTokens)} in`}
        </span>
      </div>
      {priced && percent != null ? (
        <div
          aria-label={`${row.label} spend against its limit`}
          aria-valuemax={100}
          aria-valuemin={0}
          aria-valuenow={percent}
          className="h-1 w-full overflow-hidden rounded-full bg-muted"
          role="progressbar"
        >
          <div
            className={`h-full rounded-full transition-[width] duration-500 ${ZONE_BAR[zone]}`}
            style={{ width: `${percent}%` }}
          />
        </div>
      ) : null}
    </div>
  );
}

export function SidebarUsageStrip() {
  const [series, setSeries] = React.useState<AgentUsageSeries | null>(null);
  const [collapsed, setCollapsed] = React.useState(() =>
    readJson<boolean>(COLLAPSED_STORAGE_KEY, false),
  );
  const { groups: limitGroups } = useProviderLimits();

  const load = React.useCallback(async () => {
    try {
      setSeries(
        await getAgentUsageSeries({
          bucketBoundaries: buildBucketBoundaries(1),
        }),
      );
    } catch {
      setSeries(null);
    }
  }, []);

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

  const runtimes = React.useMemo(
    () => rollupByRuntime(series?.agents ?? []),
    [series],
  );
  const dailyBudget = React.useMemo(readDailyBudget, []);
  const runtimeBudgets = React.useMemo(readRuntimeBudgets, []);

  // The binding constraint: the fullest provider window. That, not the dollar
  // figure, is what stops the team, so it is what the collapsed strip shows.
  const tightest = (limitGroups ?? [])
    .flatMap((group) => group.windows.map((limit) => ({ group, limit })))
    // A window that already reset describes nothing current, so it must not
    // become the headline — a stale 99% would keep the strip red for days
    // after the limit actually cleared.
    .filter(({ limit }) => !limit.stale)
    .reduce<{ group: LimitGroup; limit: LimitWindow } | null>(
      (worst, candidate) =>
        worst == null || candidate.limit.usedPercent > worst.limit.usedPercent
          ? candidate
          : worst,
      null,
    );

  // Nothing ran today and no limit could be read: no strip, rather than a row
  // of zeros that reads as a measurement of idleness.
  if (runtimes.length === 0 && tightest == null) return null;

  const total = runtimes.reduce<number | null>((sum, row) => {
    if (row.costUsd == null) return sum;
    return (sum ?? 0) + row.costUsd;
  }, null);
  const totalZone = budgetZone(total, dailyBudget) ?? "ok";
  const totalPercent = Math.round(budgetFraction(total, dailyBudget) * 100);

  // The tightest window, read like a battery: what is left of it.
  const headlineLeft =
    tightest == null ? null : remainingPercent(tightest.limit.usedPercent);
  const headlineZone =
    tightest == null ? totalZone : limitZone(tightest.limit.usedPercent);

  return (
    <div className="px-2 pb-2" data-testid="sidebar-usage-strip">
      <button
        aria-expanded={!collapsed}
        className="flex w-full items-center justify-between gap-2 rounded px-1 py-1 text-2xs hover:bg-sidebar-accent"
        onClick={() => {
          const next = !collapsed;
          setCollapsed(next);
          try {
            window.localStorage.setItem(
              COLLAPSED_STORAGE_KEY,
              JSON.stringify(next),
            );
          } catch {
            // A remembered fold is a convenience, never a correctness need.
          }
        }}
        type="button"
      >
        <span className="font-medium uppercase tracking-wide text-muted-foreground">
          {tightest == null ? "Usage today" : "Plan limits"}
        </span>
        <span
          className={`shrink-0 truncate tabular-nums ${
            headlineZone === "ok" ? "text-muted-foreground" : ""
          }`}
          title={
            tightest == null
              ? undefined
              : `${tightest.group.label} · ${tightest.limit.label}${
                  tightest.limit.resets
                    ? ` — resets ${tightest.limit.resets}`
                    : ""
                }`
          }
        >
          {headlineLeft == null
            ? total == null
              ? "—"
              : `${totalPercent}% · ${formatUsd(total)}`
            : `${headlineLeft}% left · ${tightest?.limit.label}`}
        </span>
      </button>
      <div className="mt-1 h-1 w-full overflow-hidden rounded-full bg-muted">
        <div
          className={`h-full rounded-full transition-[width] duration-500 ${ZONE_BAR[headlineZone]}`}
          style={{ width: `${headlineLeft ?? totalPercent}%` }}
        />
      </div>
      {collapsed ? null : (
        <div className="mt-2 space-y-2.5">
          {/*
            Plan limits first. The dollars below are what each harness says its
            turns were worth; on a subscription the ceiling that actually stops
            the team is the provider's own window, so that is the number worth
            seeing without opening anything.
          */}
          {limitGroups && limitGroups.length > 0 ? (
            <ProviderLimitBars dense groups={limitGroups} />
          ) : null}

          <div className="space-y-1.5 border-t border-border/50 pt-2">
            <p className="text-3xs font-medium uppercase tracking-wide text-muted-foreground">
              Spend
            </p>
            {runtimes.map((row) => (
              <RuntimeLine
                key={row.harness}
                limit={runtimeBudget(runtimeBudgets, row.harness)}
                row={row}
              />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
