import assert from "node:assert/strict";
import test from "node:test";

import { compactObserverEvent, MAX_KEPT_TEXT } from "./compactObserverEvent.ts";

function event(kind, payload) {
  return {
    seq: 1,
    timestamp: "t",
    kind,
    agentIndex: 0,
    channelId: null,
    sessionId: null,
    turnId: null,
    payload,
  };
}

test("a file written in full is kept to its opening, with a marker", () => {
  const file = "x".repeat(60_000);
  const before = event("acp_write", {
    method: "session/update",
    params: {
      update: { content: [{ text: file }], title: "Write ARCHITECTURE.md" },
    },
  });
  const after = compactObserverEvent(before);
  const text = after.payload.params.update.content[0].text;
  assert.ok(text.startsWith("x".repeat(MAX_KEPT_TEXT)));
  assert.match(text, /44,000 more characters not kept in the app/);
  assert.equal(after.payload.params.update.title, "Write ARCHITECTURE.md");
  assert.equal(
    before.payload.params.update.content[0].text.length,
    60_000,
    "the original is not mutated",
  );
});

test("an event with nothing oversized is passed through as the same object", () => {
  const small = event("acp_read", {
    method: "session/update",
    params: { text: "ok" },
  });
  assert.equal(compactObserverEvent(small), small);
});

test("requests the owner reviews are never cut", () => {
  const prompt = "p".repeat(20_000);
  for (const kind of [
    "agent_management_request",
    "project_channel_request",
    "folder_project_request",
    "session_config_captured",
    "control_result",
  ]) {
    const request = event(kind, { request: { systemPrompt: prompt } });
    assert.equal(compactObserverEvent(request), request, kind);
  }
});

test("raw payloads and nested arrays are trimmed too", () => {
  const raw = compactObserverEvent(
    event("raw_json_rpc", ["a", ["b".repeat(20_000)]]),
  );
  assert.equal(raw.payload[0], "a");
  assert.ok(raw.payload[1][0].length < 20_000);
});
