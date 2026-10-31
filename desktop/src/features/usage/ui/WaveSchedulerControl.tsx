import * as React from "react";
import { toast } from "sonner";

import {
  setWaveScheduler,
  waveSchedulerConfig,
  type WaveConfig,
} from "@/shared/api/tauri";

/**
 * Run the team a wave at a time instead of all at once.
 *
 * Every mentioned agent wakes immediately, so a thirty-agent team becomes
 * thirty sessions competing for one provider window. They do not go faster for
 * it — the window is the bottleneck, so crowding it makes each turn slower and
 * the limit arrive sooner. This machine's usage report measured the effect:
 * 42% of usage happened while four or more sessions ran in parallel.
 *
 * Queued agents are stopped, never deleted. They keep their identity, channel
 * membership and history, and start again as places free up.
 */
export function WaveSchedulerControl() {
  const [config, setConfig] = React.useState<WaveConfig | null>(null);
  const [busy, setBusy] = React.useState(false);

  React.useEffect(() => {
    void (async () => {
      try {
        setConfig(await waveSchedulerConfig());
      } catch {
        // The shipped defaults are safe: they are "off".
        setConfig(null);
      }
    })();
  }, []);

  async function save(next: WaveConfig) {
    setBusy(true);
    try {
      setConfig(await setWaveScheduler(next));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }

  const current: WaveConfig = config ?? {
    enabled: false,
    concurrentAgents: 4,
    idleMinutes: 10,
  };

  return (
    <div className="space-y-2.5 px-4 py-3.5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <div className="min-w-0">
          <p className="text-xs font-medium">Run in waves</p>
          <p className="text-3xs leading-relaxed text-muted-foreground">
            Cap how many agents run at once. The rest wait their turn and keep
            everything — four working agents finish sooner than fifteen
            competing for one limit.
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
              htmlFor="wave-concurrent"
            >
              Agents running at once
            </label>
            <input
              className="h-7 w-20 shrink-0 rounded border bg-background px-2 text-xs tabular-nums"
              disabled={busy}
              id="wave-concurrent"
              inputMode="numeric"
              min={1}
              onChange={(event) => {
                const next = Number(event.target.value);
                if (!Number.isFinite(next) || next < 1) return;
                void save({ ...current, concurrentAgents: next });
              }}
              step={1}
              type="number"
              value={current.concurrentAgents}
            />
          </div>

          <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
            <label
              className="text-3xs text-muted-foreground"
              htmlFor="wave-idle"
            >
              Treat an agent as finished after (minutes quiet)
            </label>
            <input
              className="h-7 w-20 shrink-0 rounded border bg-background px-2 text-xs tabular-nums"
              disabled={busy}
              id="wave-idle"
              inputMode="decimal"
              min={1}
              onChange={(event) => {
                const next = Number(event.target.value);
                if (!Number.isFinite(next) || next < 1) return;
                void save({ ...current, idleMinutes: next });
              }}
              step={5}
              type="number"
              value={current.idleMinutes}
            />
          </div>

          <p className="text-3xs leading-relaxed text-muted-foreground">
            An agent mid-turn is never stopped to make room, since cutting a
            turn off wastes everything it already spent. Coordination roles hold
            a place in every wave; among the rest, whoever has waited longest
            goes next.
          </p>
        </>
      ) : null}
    </div>
  );
}
