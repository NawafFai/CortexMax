import type { ChannelUsage } from "@/shared/api/tauriArchive";

/**
 * What the team did while nobody was watching.
 *
 * Coming back to eight active channels, the question is never "what is the
 * total" — it is "which of these moved, and did any of them go wrong". That is
 * a different shape from the usage dashboard, which answers what things cost.
 *
 * The window is bounded by a timestamp this machine writes when the digest is
 * acknowledged, rather than by any inference about presence. An app that
 * guesses when you were last "really" there gets it wrong on the day you left
 * it open overnight, and a digest that silently covers the wrong window is
 * worse than one that makes you pick.
 */

/** One channel's activity since the digest was last cleared. */
export type DigestRow = {
  channelId: string;
  /** The channel's name, or `null` when it is not among the known channels. */
  name: string | null;
  turns: number;
  agents: number;
  costUsd: number | null;
  failedTurns: number;
  lastTurnAt: number | null;
};

export type Digest = {
  since: number;
  rows: DigestRow[];
  totalTurns: number;
  totalCostUsd: number | null;
  totalFailed: number;
  /** Channels that produced turns but are not in the known channel list. */
  unknownChannels: number;
};

/** Longest window a digest will claim to cover, in seconds (7 days). */
export const MAX_DIGEST_SECONDS = 7 * 24 * 3600;

/**
 * Build the digest from a by-channel rollup and a per-agent failure count.
 *
 * Failures arrive keyed by agent, not by channel, because that is how the
 * archive records them — a turn that failed to parse kept its author but not
 * its channel. They are therefore reported as a team-wide total and never
 * split across channels, since apportioning them would be a guess presented
 * as a count.
 *
 * Channels are ordered by when they last did something, not by what they
 * spent: returning after six hours, "what moved most recently" is the question
 * being asked, and cost has a dashboard of its own.
 */
export function buildDigest({
  since,
  usage,
  names,
  failuresByAgent,
}: {
  since: number;
  usage: ChannelUsage[];
  names: Map<string, string>;
  failuresByAgent: Array<[string, number]>;
}): Digest {
  const rows: DigestRow[] = [];
  let totalTurns = 0;
  let totalCost: number | null = null;
  let unknownChannels = 0;

  for (const entry of usage) {
    if (entry.channelId == null || entry.turns === 0) continue;
    const name = names.get(entry.channelId) ?? null;
    if (name == null) unknownChannels += 1;
    totalTurns += entry.turns;
    if (entry.costUsd != null) totalCost = (totalCost ?? 0) + entry.costUsd;
    rows.push({
      channelId: entry.channelId,
      name,
      turns: entry.turns,
      agents: entry.agents,
      costUsd: entry.costUsd,
      // Held at zero deliberately: the archive cannot attribute a failed turn
      // to a channel, so a per-channel figure here would be invented.
      failedTurns: 0,
      lastTurnAt: entry.lastTurnAt,
    });
  }

  rows.sort((left, right) => (right.lastTurnAt ?? 0) - (left.lastTurnAt ?? 0));

  const totalFailed = failuresByAgent.reduce(
    (sum, [, count]) => sum + count,
    0,
  );

  return {
    since,
    rows,
    totalTurns,
    totalCostUsd: totalCost,
    totalFailed,
    unknownChannels,
  };
}

/**
 * The window a digest should cover, clamped to something it can describe.
 *
 * An unset or corrupt marker means the digest has never been cleared on this
 * machine, and a first run must not claim to summarise all of history. A
 * marker in the future — a clock change, a restored backup — is equally
 * unusable. Both fall back to the longest window the digest will claim.
 */
export function digestSince(stored: number | null, now: number): number {
  const floor = now - MAX_DIGEST_SECONDS;
  if (stored == null || !Number.isFinite(stored)) return floor;
  if (stored >= now) return floor;
  return Math.max(stored, floor);
}
