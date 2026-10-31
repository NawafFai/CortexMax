import assert from "node:assert/strict";
import test from "node:test";

import { runningPairs } from "./useFolderProjectRequests.ts";

function status(
  pubkey,
  lifecycle,
  relayUrl = "wss://example-team.communities.buzz.xyz",
) {
  return {
    pubkey,
    relayUrl,
    localSetup: true,
    lifecycle,
    pid: 1,
    error: null,
    logPath: null,
  };
}

test("only the agents up in this community are paused for a binding", () => {
  const pairs = runningPairs(
    [
      status("cortex", "ready"),
      status("pilot", "waking"),
      status("scope", "starting"),
      status("atlas", "stopped"),
      status("forge", "failed"),
      status("elsewhere", "ready", "wss://another.example"),
    ],
    "wss://example-team.communities.buzz.xyz/",
  );
  assert.deepEqual(
    pairs.map((pair) => pair.pubkey),
    ["cortex", "pilot", "scope"],
  );
});
