import * as React from "react";

import { channelBudget, formatUsd } from "@/features/usage/lib/usageWindow";
import {
  readChannelBudgets,
  readDailyBudget,
  writeChannelBudgets,
} from "@/features/usage/lib/budgetStorage";
import { Button } from "@/shared/ui/button";

/**
 * What this one project is allowed to cost in a day.
 *
 * Before this, every channel was measured against a single account-wide daily
 * budget, which stops meaning anything as soon as there is more than one
 * project: eight channels sharing $50 all read green right up until one of
 * them has spent the entire day. Giving a project its own figure makes its bar
 * answer a question about that project.
 *
 * It sits here, beside the project's deliverables and its spend, because this
 * is where the owner is already weighing what the work is worth — not in a
 * settings list that names channels by id.
 *
 * Two things it is honest about. It is a display scale, not a spending
 * control: nothing in the backend reads it, so crossing it colours a bar and
 * stops nothing. And it is stored per machine, so it does not follow the
 * account to another computer.
 */
export function ChannelBudgetControl({ channelId }: { channelId: string }) {
  const fallback = React.useMemo(readDailyBudget, []);
  const [budgets, setBudgets] = React.useState(readChannelBudgets);

  const stored = budgets[channelId];
  const hasOwn = Number.isFinite(stored) && stored >= 0;
  const effective = channelBudget(budgets, channelId, fallback);
  const [draft, setDraft] = React.useState(String(effective));

  // A different project means a different figure; re-seed rather than carry
  // the previous project's number into this one's field.
  React.useEffect(() => {
    setDraft(String(channelBudget(readChannelBudgets(), channelId, fallback)));
  }, [channelId, fallback]);

  function commit(raw: string) {
    const value = Number(raw);
    if (!Number.isFinite(value) || value < 0) return;
    const next = { ...budgets, [channelId]: value };
    setBudgets(next);
    writeChannelBudgets(next);
  }

  function clear() {
    const next = { ...budgets };
    delete next[channelId];
    setBudgets(next);
    writeChannelBudgets(next);
    setDraft(String(fallback));
  }

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <div className="min-w-0">
          <label
            className="text-sm font-medium"
            htmlFor={`channel-budget-${channelId}`}
          >
            Daily budget for this project
          </label>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Sets where this project&apos;s spend bar turns amber and then red.
            It colours a bar; it does not stop a run.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <input
            className="h-8 w-28 rounded border bg-background px-2 text-sm tabular-nums"
            id={`channel-budget-${channelId}`}
            inputMode="decimal"
            min={0}
            onBlur={(event) => commit(event.target.value)}
            onChange={(event) => setDraft(event.target.value)}
            step={5}
            type="number"
            value={draft}
          />
          {hasOwn ? (
            <Button onClick={clear} size="sm" type="button" variant="outline">
              Reset
            </Button>
          ) : null}
        </div>
      </div>
      <p className="text-3xs leading-relaxed text-muted-foreground">
        {hasOwn
          ? effective > 0
            ? `This project is measured against ${formatUsd(effective)} a day.`
            : "Set to zero, so this project's spend bar is hidden rather than scaled."
          : `No budget of its own, so it follows the account-wide ${formatUsd(fallback)} a day.`}
      </p>
    </div>
  );
}
