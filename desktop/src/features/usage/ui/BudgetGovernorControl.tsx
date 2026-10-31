import * as React from "react";
import { toast } from "sonner";

import {
  budgetGovernorState,
  setBudgetGovernor,
  type GovernorAction,
  type GovernorConfig,
  type GovernorRecord,
} from "@/shared/api/tauri";

/**
 * The governor: act on a provider limit before it stops the team for you.
 *
 * Seeing a window at 99% only helps if someone is looking. A run that starts
 * overnight and exhausts a weekly cap by morning is not prevented by a bar in
 * the sidebar. This sets one action to take when a window crosses a threshold.
 *
 * It ships off. Turning it on with the default action changes nothing but the
 * notification, because moving a team to cheaper models — let alone stopping
 * it — is the owner's decision to make deliberately.
 */

const ACTIONS: { value: GovernorAction; label: string; detail: string }[] = [
  {
    value: "notify",
    label: "Tell me",
    detail: "Raise a notice and change nothing.",
  },
  {
    value: "economy",
    label: "Drop to Economy",
    detail: "Move the whole team to the cheapest model tier.",
  },
  {
    value: "break",
    label: "Stop everything",
    detail: "Break every running agent. Start stays yours to press.",
  },
];

export function BudgetGovernorControl() {
  const [record, setRecord] = React.useState<GovernorRecord | null>(null);
  const [busy, setBusy] = React.useState(false);

  React.useEffect(() => {
    void (async () => {
      try {
        setRecord(await budgetGovernorState());
      } catch {
        // Not reading the record must not hide the control; a fresh default is
        // safe because the default is off.
        setRecord(null);
      }
    })();
  }, []);

  async function save(next: GovernorConfig) {
    setBusy(true);
    try {
      setRecord(await setBudgetGovernor(next));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }

  const config: GovernorConfig = record?.config ?? {
    enabled: false,
    thresholdPercent: 90,
    action: "notify",
  };

  return (
    <div className="space-y-2.5 px-4 py-3.5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <div className="min-w-0">
          <p className="text-xs font-medium">Governor</p>
          <p className="text-3xs text-muted-foreground">
            Act automatically when a plan limit crosses a threshold, instead of
            finding out the next morning.
          </p>
        </div>
        <label className="flex shrink-0 items-center gap-2 text-xs">
          <input
            checked={config.enabled}
            disabled={busy}
            onChange={(event) =>
              void save({ ...config, enabled: event.target.checked })
            }
            type="checkbox"
          />
          <span>{config.enabled ? "On" : "Off"}</span>
        </label>
      </div>

      {config.enabled ? (
        <>
          <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
            <label
              className="text-3xs text-muted-foreground"
              htmlFor="governor-threshold"
            >
              Act when any window reaches (%)
            </label>
            <input
              className="h-7 w-20 shrink-0 rounded border bg-background px-2 text-xs tabular-nums"
              disabled={busy}
              id="governor-threshold"
              inputMode="numeric"
              max={100}
              min={1}
              onChange={(event) => {
                const next = Number(event.target.value);
                if (!Number.isFinite(next) || next < 1 || next > 100) return;
                void save({ ...config, thresholdPercent: next });
              }}
              step={5}
              type="number"
              value={config.thresholdPercent}
            />
          </div>

          <div className="grid gap-1.5 [@container(min-width:34rem)]:grid-cols-3">
            {ACTIONS.map((action) => {
              const active = config.action === action.value;
              return (
                <button
                  aria-pressed={active}
                  className={`rounded-lg border px-3 py-2 text-left transition-colors ${
                    active
                      ? "border-foreground/30 bg-muted"
                      : "border-border/70 hover:bg-muted/60"
                  }`}
                  disabled={busy}
                  key={action.value}
                  onClick={() => void save({ ...config, action: action.value })}
                  type="button"
                >
                  <span className="block text-xs font-medium">
                    {action.label}
                  </span>
                  <span className="mt-0.5 block text-3xs leading-relaxed text-muted-foreground">
                    {action.detail}
                  </span>
                </button>
              );
            })}
          </div>

          <p className="text-3xs leading-relaxed text-muted-foreground">
            A crossing acts once, so pressing Start after an automatic stop will
            not be met with another. The governor never resumes for you, and a
            reading from a window that already reset can never trigger it.
          </p>
        </>
      ) : null}

      {record?.lastFiredAt ? (
        <p className="rounded border border-border/60 bg-muted/40 px-2.5 py-2 text-3xs leading-relaxed">
          <span className="font-medium">Last acted</span>{" "}
          {new Date(record.lastFiredAt).toLocaleString()} — {record.lastReason}.{" "}
          {record.lastOutcome}
        </p>
      ) : null}
    </div>
  );
}
