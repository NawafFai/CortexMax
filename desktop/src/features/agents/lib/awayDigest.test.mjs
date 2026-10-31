import assert from "node:assert/strict";
import { test } from "node:test";

import { buildDigest, digestSince, MAX_DIGEST_SECONDS } from "./awayDigest.ts";

function usageRow(channelId, turns, cost, lastTurnAt, agents = 1) {
  return {
    channelId,
    turns,
    inputTokens: 0,
    outputTokens: 0,
    costUsd: cost,
    turnsWithoutCost: 0,
    unparsedTurns: 0,
    agents,
    lastTurnAt,
  };
}

const NAMES = new Map([
  ["c1", "Case study"],
  ["c2", "Code review"],
]);

test("channels are ordered by what moved most recently, not by cost", () => {
  const digest = buildDigest({
    since: 100,
    usage: [usageRow("c1", 5, 90, 200), usageRow("c2", 5, 1, 900)],
    names: NAMES,
    failuresByAgent: [],
  });
  assert.equal(digest.rows[0].name, "Code review", "newest activity leads");
  assert.equal(digest.rows[1].name, "Case study");
});

test("totals add turns and cost across channels", () => {
  const digest = buildDigest({
    since: 100,
    usage: [usageRow("c1", 5, 2.5, 200), usageRow("c2", 7, 1.5, 300)],
    names: NAMES,
    failuresByAgent: [],
  });
  assert.equal(digest.totalTurns, 12);
  assert.equal(digest.totalCostUsd, 4);
});

test("a channel with no turns is not a line in the digest", () => {
  const digest = buildDigest({
    since: 100,
    usage: [usageRow("c1", 0, null, null), usageRow("c2", 3, 1, 300)],
    names: NAMES,
    failuresByAgent: [],
  });
  assert.equal(digest.rows.length, 1);
  assert.equal(digest.rows[0].channelId, "c2");
});

// The archive keeps a failed turn's author but not its channel, so splitting
// failures across channels would be a guess presented as a count.
test("failures are a team total and are never apportioned to channels", () => {
  const digest = buildDigest({
    since: 100,
    usage: [usageRow("c1", 5, 1, 200), usageRow("c2", 5, 1, 300)],
    names: NAMES,
    failuresByAgent: [
      ["agent-a", 3],
      ["agent-b", 2],
    ],
  });
  assert.equal(digest.totalFailed, 5);
  for (const row of digest.rows) {
    assert.equal(row.failedTurns, 0, "no channel may claim a failure count");
  }
});

test("a channel the machine can no longer name is counted, not dropped", () => {
  const digest = buildDigest({
    since: 100,
    usage: [usageRow("gone", 4, 1, 200)],
    names: NAMES,
    failuresByAgent: [],
  });
  assert.equal(digest.rows.length, 1);
  assert.equal(digest.rows[0].name, null);
  assert.equal(digest.unknownChannels, 1);
});

test("an unpriced channel leaves the total unpriced rather than counting zero", () => {
  const digest = buildDigest({
    since: 100,
    usage: [usageRow("c1", 5, null, 200)],
    names: NAMES,
    failuresByAgent: [],
  });
  assert.equal(digest.totalCostUsd, null);
});

// A first run must not claim to summarise all of history.
test("no stored marker covers the longest window the digest will claim", () => {
  const now = 1_000_000;
  assert.equal(digestSince(null, now), now - MAX_DIGEST_SECONDS);
  assert.equal(digestSince(Number.NaN, now), now - MAX_DIGEST_SECONDS);
});

test("a marker in the future is unusable and falls back", () => {
  const now = 1_000_000;
  assert.equal(digestSince(now + 500, now), now - MAX_DIGEST_SECONDS);
  assert.equal(digestSince(now, now), now - MAX_DIGEST_SECONDS);
});

test("a marker older than the cap is clamped, not honoured", () => {
  const now = 1_000_000;
  assert.equal(digestSince(1, now), now - MAX_DIGEST_SECONDS);
});

test("a recent marker is used as-is", () => {
  const now = 1_000_000;
  assert.equal(digestSince(now - 3600, now), now - 3600);
});
