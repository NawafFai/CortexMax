import * as React from "react";
import { RefreshCw } from "lucide-react";

import { getProviderLimits } from "@/shared/api/tauri";
import { stripGroups } from "@/features/usage/lib/stripVisibility";
import type { LimitGroup, LimitWindow } from "@/shared/api/tauri";
import {
  forecastExhaustion,
  formatCountdown,
  formatResetInstant,
  limitZone,
  remainingPercent,
} from "@/features/usage/lib/usageWindow";

/**
 * The provider ceilings that actually stop a run.
 *
 * The spend figures elsewhere are what each harness reports its own turns were
 * worth; on a subscription that is API-equivalent value, not a bill. The thing
 * that genuinely halts the team mid-task is the provider's own window — Claude's
 * 5-hour and weekly caps, Codex's weekly cap — and those were only visible by
 * leaving the app.
 *
 * Nothing here is estimated. Each number is the one that runtime reports for
 * itself, and a runtime that cannot be read says so rather than drawing an
 * empty bar, because 0% and "unknown" look identical on a bar and mean the
 * opposite thing.
 */

const ZONE_BAR: Record<string, string> = {
  ok: "bg-emerald-500",
  warn: "bg-amber-500",
  over: "bg-red-500",
};

const ZONE_TEXT: Record<string, string> = {
  ok: "text-muted-foreground",
  warn: "text-amber-600 dark:text-amber-400",
  over: "text-red-600 dark:text-red-400",
};

/**
 * A live countdown to `resetsAt`, or null when the provider gave no instant.
 *
 * It ticks every second only while the reset is under an hour away, which is
 * when the seconds are what the owner is actually watching. Further out it
 * ticks once a minute, because re-rendering every bar every second to change
 * "2d 4h" into "2d 4h" is pure waste.
 */
function useCountdown(resetsAt: number | null): string | null {
  const [now, setNow] = React.useState(() => Date.now() / 1000);

  const remaining = resetsAt == null ? null : resetsAt - now;
  const urgent = remaining != null && remaining < 3600;

  React.useEffect(() => {
    if (resetsAt == null) return;
    const period = urgent ? 1000 : 60_000;
    const timer = window.setInterval(() => setNow(Date.now() / 1000), period);
    return () => {
      window.clearInterval(timer);
    };
  }, [resetsAt, urgent]);

  return remaining == null ? null : formatCountdown(remaining);
}

/** A limit is measured against its own 100%, so the zones read as fractions. */
function zoneFor(usedPercent: number): string {
  return limitZone(usedPercent);
}

/** Poll the provider limits, sharing the backend's cache between mounts. */
export function useProviderLimits(pollMs = 120_000) {
  const [groups, setGroups] = React.useState<LimitGroup[] | null>(null);
  const [refreshing, setRefreshing] = React.useState(false);

  const load = React.useCallback(async (refresh = false) => {
    if (refresh) setRefreshing(true);
    try {
      setGroups(await getProviderLimits(refresh));
    } catch {
      // A failed read must not blank a panel that was showing real numbers a
      // moment ago; the previous reading stays until one succeeds.
    } finally {
      if (refresh) setRefreshing(false);
    }
  }, []);

  React.useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), pollMs);
    return () => {
      window.clearInterval(timer);
    };
  }, [load, pollMs]);

  return { groups, refreshing, refresh: () => void load(true) };
}

function WindowRow({
  group,
  window: limit,
  dense,
}: {
  group: LimitGroup;
  window: LimitWindow;
  dense: boolean;
}) {
  // Shown as what is left, like a battery: a full window reads 100% in green.
  const left = remainingPercent(limit.usedPercent);
  const zone = limit.stale ? "ok" : zoneFor(limit.usedPercent);
  const countdown = useCountdown(limit.resetsAt);
  // A window already past its reset has no future instant to state.
  const resetLine = limit.stale
    ? null
    : limit.resetsAt != null
      ? formatResetInstant(limit.resetsAt)
      : limit.resets;
  // Only drawn when the current rate would spend the window before it resets;
  // a window that will last needs no warning.
  const forecast = limit.stale
    ? null
    : forecastExhaustion({
        usedPercent: limit.usedPercent,
        resetsAt: limit.resetsAt,
        windowMinutes: limit.windowMinutes,
        now: Date.now() / 1000,
      });
  return (
    <div
      className="space-y-1"
      // In the sidebar the reset time has nowhere to go, so it lives here.
      title={
        limit.stale
          ? `${group.label} · ${limit.label} — the window reset at ${limit.resets}; its last reading was ${left}% left, and ${group.label} reports a new one on its next turn`
          : limit.resets
            ? `${group.label} · ${limit.label} — resets ${limit.resets}`
            : `${group.label} · ${limit.label}`
      }
    >
      <div
        className={`flex items-baseline justify-between gap-2 ${dense ? "text-2xs" : "text-xs"}`}
      >
        <span className="min-w-0 truncate">
          {dense ? null : (
            <span className="text-muted-foreground">{group.label} · </span>
          )}
          {limit.label}
        </span>
        {/*
          The window name is the thing that must stay readable: a sidebar row
          is ~250px, and letting the reset time share it truncated "Week ·
          Fable" to "W…". Wide layouts have room for both; narrow ones keep the
          name and move the time to the row's tooltip.
        */}
        {limit.stale ? (
          <span className="shrink-0 text-muted-foreground">
            reset · awaiting next turn
          </span>
        ) : (
          <span
            className={`shrink-0 tabular-nums font-medium ${ZONE_TEXT[zone]}`}
          >
            {left}% left
            {/*
              A countdown answers "how long have I got" without arithmetic,
              which is the question a deadline is actually asked. The day and
              hour it comes back get their own line under the bar.
            */}
            {countdown ? (
              <span className="ml-1.5 font-normal text-muted-foreground">
                · {countdown}
              </span>
            ) : null}
          </span>
        )}
      </div>
      {/*
        The fill is what is left. A window that has already reset is full
        again, whatever its last reading said: painting that reading would say
        the owner is still short when they are not. The old number is kept in
        the tooltip rather than thrown away.
      */}
      <div
        aria-label={`${group.label} ${limit.label} remaining`}
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={limit.stale ? 100 : left}
        className={`w-full overflow-hidden rounded-full bg-muted ${dense ? "h-1" : "h-1.5"}`}
        role="progressbar"
      >
        <div
          className={`h-full rounded-full transition-[width] duration-500 ${ZONE_BAR[zone]}`}
          style={{ width: limit.stale ? "100%" : `${left}%` }}
        />
      </div>

      {/*
        When, not only how long: the line the provider's own usage page
        leads with. Kept under the bar rather than beside the percentage, so
        a narrow sidebar row never trades the window's name for it.
      */}
      {resetLine ? (
        <p
          className={`${dense ? "text-3xs" : "text-2xs"} text-muted-foreground`}
        >
          Resets {resetLine}
        </p>
      ) : null}

      {/*
        A projection, not a measurement: it carries the current burn forward
        unchanged, which is why it says "at this rate" and why it is withheld
        until enough of the window has elapsed to mean anything.
      */}
      {forecast ? (
        <p
          className={`${dense ? "text-3xs" : "text-2xs"} leading-relaxed text-amber-600 dark:text-amber-400`}
        >
          At this rate, spent in {formatCountdown(forecast.inSeconds)} — before
          it resets.
        </p>
      ) : null}
    </div>
  );
}

/** Every runtime's windows, one bar each. */
export function ProviderLimitBars({
  groups,
  dense = false,
}: {
  groups: LimitGroup[];
  dense?: boolean;
}) {
  /*
    The strip carries only the runtimes that actually report a window.
    A provider with nothing to show still has a reason worth reading —
    Kimi keeps its figures inside its own CLI, OpenCode publishes no plan
    limit at all — but a paragraph of explanation in a sidebar strip costs
    the space the real numbers need and says nothing an owner can act on at
    a glance. The reasons stay in the Usage dashboard, which has room for
    them; the strip answers one question only: how close am I to a ceiling.
  */
  const shown = dense ? stripGroups(groups) : groups;

  if (shown.length === 0) return null;

  return (
    <div className={dense ? "space-y-2" : "space-y-3"}>
      {shown.map((group) => (
        <div
          className={dense ? "space-y-1.5" : "space-y-2"}
          key={group.runtime}
        >
          {dense ? (
            <div className="flex items-baseline justify-between gap-2 text-3xs text-muted-foreground">
              <span className="truncate font-medium uppercase tracking-wide">
                {group.label}
              </span>
              {group.note ? (
                <span className="shrink-0 truncate">{group.note}</span>
              ) : null}
            </div>
          ) : null}

          {group.unavailable ? (
            <p
              className={`text-muted-foreground ${dense ? "text-3xs" : "text-xs"}`}
            >
              {dense ? "" : `${group.label}: `}
              {group.unavailable}
            </p>
          ) : (
            group.windows.map((limit) => (
              <WindowRow
                dense={dense}
                group={group}
                key={`${group.runtime}:${limit.label}`}
                window={limit}
              />
            ))
          )}
        </div>
      ))}
    </div>
  );
}

/** The Usage page section: the same bars, with a manual refresh. */
export function ProviderLimitsSection() {
  const { groups, refreshing, refresh } = useProviderLimits();

  return (
    <div className="space-y-2.5 px-4 py-3.5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <div className="min-w-0">
          <p className="text-xs font-medium">Plan limits</p>
          <p className="text-3xs text-muted-foreground">
            The ceilings that actually stop a run, as each provider reports
            them.
          </p>
        </div>
        <button
          aria-label="Re-read the provider limits now"
          className="flex shrink-0 items-center gap-1.5 rounded border px-2 py-1 text-3xs text-muted-foreground transition-colors hover:bg-muted/60"
          disabled={refreshing}
          onClick={refresh}
          type="button"
        >
          <RefreshCw className={refreshing ? "animate-spin" : ""} size={11} />
          <span>{refreshing ? "Reading…" : "Refresh"}</span>
        </button>
      </div>

      {groups == null ? (
        <p className="text-xs text-muted-foreground">Reading plan limits…</p>
      ) : (
        <ProviderLimitBars groups={groups} />
      )}

      <p className="text-3xs leading-relaxed text-muted-foreground">
        Both are asked for your own account as it stands now: Claude through its
        CLI, Codex through its own app-server. If Codex cannot be reached, the
        last reading in its session log is shown until its next turn.
      </p>
    </div>
  );
}
