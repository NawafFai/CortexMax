import * as React from "react";

import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";
import { useChannelsQuery } from "@/features/channels/hooks";
import { formatAgo } from "@/features/agents/lib/teamRoster";
import {
  buildDigest,
  digestSince,
  type Digest,
} from "@/features/agents/lib/awayDigest";
import { formatUsd } from "@/features/usage/lib/usageWindow";
import {
  getInvalidTurnsByAgent,
  getUsageByChannel,
} from "@/shared/api/tauriArchive";
import { Button } from "@/shared/ui/button";

/**
 * What happened while you were away.
 *
 * Returning to eight running channels, the dashboards answer what things cost
 * and the roster answers who is up — neither answers "which of these moved,
 * and did anything go wrong", which is the only question on the way back to
 * the desk. Reading eight channels to find out is how a failed run goes
 * unnoticed for a day.
 *
 * "Away" is a timestamp this machine writes when you clear the digest, not a
 * guess at when you were last really present. Guessing gets it wrong the night
 * you leave the app open, and a digest quietly covering the wrong window is
 * worse than one that asks you to mark it read.
 *
 * Failures are shown as a team total rather than per channel, because the
 * archive keeps a failed turn's author but not its channel. Splitting them
 * across channels would be a guess wearing the clothes of a count.
 */

const SINCE_KEY = "cortex-max.away-digest-since";

function readSince(): number | null {
  try {
    const raw = window.localStorage.getItem(SINCE_KEY);
    if (raw == null) return null;
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function writeSince(value: number): void {
  try {
    window.localStorage.setItem(SINCE_KEY, String(value));
  } catch {
    // The marker is a convenience; losing it widens the window, never breaks.
  }
}

export function AwayDigestCard() {
  const channelsQuery = useChannelsQuery();
  const [digest, setDigest] = React.useState<Digest | null>(null);
  const [loading, setLoading] = React.useState(true);

  const names = React.useMemo(() => {
    const map = new Map<string, string>();
    for (const channel of channelsQuery.data ?? []) {
      if (channel.id && channel.name) map.set(channel.id, channel.name);
    }
    return map;
  }, [channelsQuery.data]);

  const load = React.useCallback(async () => {
    const now = Math.floor(Date.now() / 1000);
    const since = digestSince(readSince(), now);
    try {
      const [usage, failures] = await Promise.all([
        getUsageByChannel(since, now),
        getInvalidTurnsByAgent(since, now).catch(() => []),
      ]);
      setDigest(
        buildDigest({ since, usage, names, failuresByAgent: failures }),
      );
    } catch {
      // No archive yet is the ordinary state on a fresh machine; an empty
      // digest reads correctly, an error banner does not.
      setDigest(null);
    } finally {
      setLoading(false);
    }
  }, [names]);

  React.useEffect(() => {
    void load();
  }, [load]);

  function clear() {
    writeSince(Math.floor(Date.now() / 1000));
    setLoading(true);
    void load();
  }

  if (loading && digest == null) return null;
  // Nothing ran: no card, rather than a summary of an empty window that reads
  // as though the team were checked and found idle.
  if (digest == null || digest.rows.length === 0) return null;

  const elapsed = Math.floor(Date.now() / 1000) - digest.since;

  return (
    <SettingsOptionGroup title="While you were away">
      <div className="space-y-3 px-4 py-3.5">
        <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
          <p className="text-xs text-muted-foreground">
            {digest.totalTurns} turn{digest.totalTurns === 1 ? "" : "s"} across{" "}
            {digest.rows.length} channel{digest.rows.length === 1 ? "" : "s"} in
            the last {formatAgo(elapsed)}
            {digest.totalCostUsd == null
              ? ""
              : `, costing ${formatUsd(digest.totalCostUsd)}`}
            .
          </p>
          <Button onClick={clear} size="sm" type="button" variant="outline">
            Mark read
          </Button>
        </div>

        {digest.totalFailed > 0 ? (
          <p className="text-xs font-medium text-red-600 dark:text-red-400">
            {digest.totalFailed} turn{digest.totalFailed === 1 ? "" : "s"}{" "}
            failed in this window — see Agent activity for which agent.
          </p>
        ) : null}

        <ul className="-mx-4 divide-y divide-border/55 border-t border-border/55">
          {digest.rows.map((row) => (
            <li
              className="flex items-baseline gap-3 px-4 py-2 text-xs"
              key={row.channelId}
            >
              <span className="min-w-0 flex-1 truncate font-medium">
                {row.name ?? "A channel that is no longer listed"}
              </span>
              <span className="shrink-0 tabular-nums text-muted-foreground">
                {row.turns} turn{row.turns === 1 ? "" : "s"} · {row.agents}{" "}
                agent{row.agents === 1 ? "" : "s"}
                {row.costUsd == null ? "" : ` · ${formatUsd(row.costUsd)}`}
              </span>
              <span className="w-20 shrink-0 text-right tabular-nums text-muted-foreground">
                {row.lastTurnAt == null
                  ? "—"
                  : formatAgo(Math.floor(Date.now() / 1000) - row.lastTurnAt)}
              </span>
            </li>
          ))}
        </ul>

        {digest.unknownChannels > 0 ? (
          <p className="text-3xs leading-relaxed text-muted-foreground">
            {digest.unknownChannels} of these are channels this machine can no
            longer list, so they are shown by their activity alone.
          </p>
        ) : null}
      </div>
    </SettingsOptionGroup>
  );
}
