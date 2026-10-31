import assert from "node:assert/strict";
import test from "node:test";

import { modelFamily, runtimeFamily } from "./runtimeFamily.ts";

test("the model names the family whatever harness runs it", () => {
  // Prism: a Codex harness pointed at Kimi spends Kimi's quota.
  assert.equal(runtimeFamily("codex", null, "kimi-code/k3-256k"), "kimi");
  // Judge: a Codex harness with a Claude model.
  assert.equal(runtimeFamily("codex", null, "claude-sonnet-5"), "claude");
  assert.equal(
    runtimeFamily("opencode", null, "opencode/big-pickle"),
    "opencode",
  );
  assert.equal(runtimeFamily("claude", null, "claude-opus-5-5"), "claude");
  assert.equal(runtimeFamily("codex", null, "gpt-5.6-terra"), "codex");
});

test("with no model, the harness id answers", () => {
  assert.equal(runtimeFamily("claude-agent-acp", null, null), "claude");
  assert.equal(runtimeFamily("codex", null, null), "codex");
  assert.equal(runtimeFamily("kimi", null, ""), "kimi");
});

test("with no harness id either, the launch command answers", () => {
  assert.equal(runtimeFamily(null, "/Users/me/.kimi-code/bin/kimi acp"), "kimi");
  assert.equal(
    runtimeFamily(null, "/Users/me/.local/bin/opencode acp"),
    "opencode",
  );
  assert.equal(runtimeFamily(null, "/tmp/some-tool"), null);
});

test("a model id that names no family is not guessed at", () => {
  assert.equal(modelFamily("my-local-llama"), null);
  assert.equal(modelFamily(null), null);
});
