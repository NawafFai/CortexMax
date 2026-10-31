import * as React from "react";
import { listen } from "@tauri-apps/api/event";

import { notifyAgentMetricsChanged } from "@/shared/api/tauriArchive";

/**
 * How long a burst of metric writes is gathered into one notification.
 *
 * Every mounted usage view — the sidebar strip, each channel's bar, each
 * member card, the dashboard — re-queries the archive on every notification.
 * A team of eight at work writes metric batches back to back, so a view per
 * write kept the window busy re-running the same queries against an archive
 * of hundreds of megabytes. Spend moves over minutes; five seconds late costs
 * nothing an owner can see.
 */
export const METRICS_COALESCE_MS = 5_000;

/**
 * A trailing throttle: the first call schedules one notify, calls inside the
 * window are absorbed into it, and the next call after it fires schedules the
 * next. Returns the trigger and a disposer.
 */
export function coalesced(
  notify: () => void,
  windowMs: number,
  schedule: (run: () => void, ms: number) => unknown = setTimeout,
  cancel: (handle: unknown) => void = (handle) =>
    clearTimeout(handle as ReturnType<typeof setTimeout>),
): { trigger: () => void; dispose: () => void } {
  let pending: unknown = null;
  return {
    trigger: () => {
      if (pending !== null) return;
      pending = schedule(() => {
        pending = null;
        notify();
      }, windowMs);
    },
    dispose: () => {
      if (pending !== null) cancel(pending);
      pending = null;
    },
  };
}

/**
 * Bridges the backend's `archive-agent-metrics-changed` event to the existing
 * in-process notifier.
 *
 * The archive batch that persists agent-metric rows now runs entirely in Rust,
 * so the notifier's archive-side producer became a Tauri event. Consumers
 * (`useAgentUsageSeries`) keep subscribing to `onAgentMetricsChanged` exactly
 * as before — the source moved, the contract did not. Bursts are gathered
 * into one notification (see {@link METRICS_COALESCE_MS}).
 */
export function useArchiveAgentMetricsBridge(): void {
  React.useEffect(() => {
    const burst = coalesced(notifyAgentMetricsChanged, METRICS_COALESCE_MS);
    const unlisten = listen("archive-agent-metrics-changed", burst.trigger);
    return () => {
      burst.dispose();
      void unlisten.then((fn) => fn());
    };
  }, []);
}
