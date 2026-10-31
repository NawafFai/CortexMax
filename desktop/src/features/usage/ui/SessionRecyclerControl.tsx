import * as React from "react";
import { toast } from "sonner";

import {
  sessionRecyclerConfig,
  setSessionRecycler,
  type RecyclerConfig,
} from "@/shared/api/tauri";

/**
 * Recycle long-lived sessions, so a day's work does not cost more each hour
 * than it did the last.
 *
 * An ACP session accumulates: every turn re-sends the conversation so far, so
 * an agent up since morning pays for the whole morning on each afternoon turn.
 * Restarting it gives it a fresh session and drops its context back to the
 * system prompt; the team's work is not lost, because it lives in the channel.
 *
 * An agent is only recycled when it is both old and quiet, so a turn in
 * progress is never cut off.
 */
export function SessionRecyclerControl() {
  const [config, setConfig] = React.useState<RecyclerConfig | null>(null);
  const [busy, setBusy] = React.useState(false);

  React.useEffect(() => {
    void (async () => {
      try {
        setConfig(await sessionRecyclerConfig());
      } catch {
        // Falling back to the shipped defaults is safe: they are "off".
        setConfig(null);
      }
    })();
  }, []);

  async function save(next: RecyclerConfig) {
    setBusy(true);
    try {
      setConfig(await setSessionRecycler(next));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }

  const current: RecyclerConfig = config ?? {
    enabled: false,
    maxSessionHours: 8,
    minIdleMinutes: 20,
  };

  return (
    <div className="space-y-2.5 px-4 py-3.5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <div className="min-w-0">
          <p className="text-xs font-medium">Recycle long sessions</p>
          <p className="text-3xs leading-relaxed text-muted-foreground">
            A long session re-sends its whole history every turn, so the same
            work costs more as the day goes on. Restarting an idle agent resets
            its context; its history stays in the channel.
          </p>
        </div>
        <label className="flex shrink-0 items-center gap-2 text-xs">
          <input
            checked={current.enabled}
            disabled={busy}
            onChange={(event) =>
              void save({ ...current, enabled: event.target.checked })
            }
            type="checkbox"
          />
          <span>{current.enabled ? "On" : "Off"}</span>
        </label>
      </div>

      {current.enabled ? (
        <>
          <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
            <label
              className="text-3xs text-muted-foreground"
              htmlFor="recycler-hours"
            >
              Recycle a session older than (hours)
            </label>
            <input
              className="h-7 w-20 shrink-0 rounded border bg-background px-2 text-xs tabular-nums"
              disabled={busy}
              id="recycler-hours"
              inputMode="decimal"
              min={1}
              onChange={(event) => {
                const next = Number(event.target.value);
                if (!Number.isFinite(next) || next < 1) return;
                void save({ ...current, maxSessionHours: next });
              }}
              step={1}
              type="number"
              value={current.maxSessionHours}
            />
          </div>

          <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
            <label
              className="text-3xs text-muted-foreground"
              htmlFor="recycler-idle"
            >
              ...but only after it has been quiet for (minutes)
            </label>
            <input
              className="h-7 w-20 shrink-0 rounded border bg-background px-2 text-xs tabular-nums"
              disabled={busy}
              id="recycler-idle"
              inputMode="decimal"
              min={1}
              onChange={(event) => {
                const next = Number(event.target.value);
                if (!Number.isFinite(next) || next < 1) return;
                void save({ ...current, minIdleMinutes: next });
              }}
              step={5}
              type="number"
              value={current.minIdleMinutes}
            />
          </div>

          <p className="text-3xs leading-relaxed text-muted-foreground">
            An agent still taking turns is never recycled, however old its
            session. Neither is one whose last turn cannot be read — unknown is
            not the same as idle.
          </p>
        </>
      ) : null}
    </div>
  );
}
