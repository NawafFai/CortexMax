import * as React from "react";

import type { AgentUsageSeries } from "@/shared/api/tauriArchive";
import {
  formatTokens,
  formatUsd,
  MIN_TURNS_TO_COMPARE,
  rollupByModel,
  type UsageWindowDays,
} from "@/features/usage/lib/usageWindow";

/**
 * What each model actually costs per turn, side by side.
 *
 * The team runs across a dozen models, and until now there was no way to rank
 * them against each other — including for me, which is why the advice about
 * which model to put where had been judgement rather than evidence. A week of
 * ordinary running answers it from the archive that was already being written.
 *
 * Cost per turn is the column that matters. A model's total says mostly how
 * much work it was given; only the per-turn rate says whether it is expensive.
 * Output per turn sits beside it because together they separate "costs more
 * per token" from "writes more per turn" — different problems, different fixes.
 *
 * Two things are deliberately absent. There is no failure rate per model: a
 * turn that fails to parse is archived with no model recorded, so the data
 * cannot attribute it, and inventing an attribution would be worse than the
 * gap. And a model under {@link MIN_TURNS_TO_COMPARE} turns is shown but never
 * ranked, because an average over a handful of turns reorders itself hourly
 * and invites a decision the numbers do not support.
 */

/** Cost per turn at the precision that distinguishes cheap models. */
function formatPerTurn(value: number | null): string {
  if (value == null) return "—";
  if (value === 0) return "$0";
  if (value < 0.01) return `$${value.toFixed(4)}`;
  return `$${value.toFixed(3)}`;
}

export function ModelComparisonCard({
  series,
  windowDays,
}: {
  series: AgentUsageSeries | null;
  windowDays: UsageWindowDays;
}) {
  const rows = React.useMemo(
    () => rollupByModel(series?.agents ?? []),
    [series],
  );

  if (rows.length === 0) return null;

  const comparable = rows.filter((row) => row.comparable);
  const priced = comparable.filter((row) => row.costPerTurn != null);
  // Only worth naming a cheapest and dearest when there are two to compare.
  const spread =
    priced.length >= 2
      ? { dearest: priced[0], cheapest: priced[priced.length - 1] }
      : null;

  return (
    <div className="px-4 py-3.5">
      <h4 className="text-xs font-medium text-muted-foreground">
        By model · {windowDays} days
      </h4>

      {spread ? (
        <p className="mt-1.5 text-3xs leading-relaxed text-muted-foreground">
          {spread.dearest.model} costs{" "}
          {formatPerTurn(spread.dearest.costPerTurn)} a turn against{" "}
          {formatPerTurn(spread.cheapest.costPerTurn)} for{" "}
          {spread.cheapest.model}
          {spread.dearest.costPerTurn != null &&
          spread.cheapest.costPerTurn != null &&
          spread.cheapest.costPerTurn > 0
            ? ` — ${Math.round(spread.dearest.costPerTurn / spread.cheapest.costPerTurn)}× the rate`
            : ""}
          .
        </p>
      ) : null}

      <ul className="-mx-4 mt-2.5 divide-y divide-border/55 border-t border-border/55">
        {rows.map((row) => (
          <li
            className={`flex items-baseline gap-3 px-4 py-2 text-xs ${
              row.comparable ? "" : "opacity-60"
            }`}
            key={row.key}
          >
            <div className="min-w-0 flex-1">
              <p className="truncate font-medium">{row.model}</p>
              <p className="mt-0.5 text-3xs text-muted-foreground">
                {row.runtimeLabel} · {row.agents} agent
                {row.agents === 1 ? "" : "s"} · {row.turns} turn
                {row.turns === 1 ? "" : "s"}
                {row.comparable
                  ? ""
                  : ` · under ${MIN_TURNS_TO_COMPARE}, not ranked`}
              </p>
            </div>
            <div className="shrink-0 text-right tabular-nums">
              <p className="font-medium">{formatPerTurn(row.costPerTurn)}</p>
              <p className="mt-0.5 text-3xs text-muted-foreground">per turn</p>
            </div>
            <div className="hidden shrink-0 text-right tabular-nums sm:block">
              <p>{formatUsd(row.costUsd)}</p>
              <p className="mt-0.5 text-3xs text-muted-foreground">
                {row.outputPerTurn == null
                  ? "—"
                  : `${formatTokens(Math.round(row.outputPerTurn))} out/turn`}
              </p>
            </div>
          </li>
        ))}
      </ul>

      <p className="mt-2.5 text-3xs leading-relaxed text-muted-foreground">
        Cost is what each harness reported for its own turns, so models on a
        harness that reports no price show turns without a rate. Failed turns
        are archived without a model, so this cannot show a failure rate per
        model — the per-agent failure counts on the Agents page are the nearest
        measured answer.
      </p>
    </div>
  );
}
