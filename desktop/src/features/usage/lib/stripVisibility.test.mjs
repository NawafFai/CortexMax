import assert from "node:assert/strict";
import { test } from "node:test";

import { stripGroups } from "./stripVisibility.ts";

function group(runtime, windows, unavailable = null) {
  return {
    runtime,
    label: runtime,
    windows,
    plan: null,
    note: null,
    unavailable,
    ageSeconds: null,
  };
}

const window = {
  label: "Week",
  usedPercent: 40,
  resets: null,
  resetsAt: null,
  windowMinutes: 10080,
  stale: false,
  model: null,
};

test("a runtime that reports a window is shown", () => {
  assert.equal(stripGroups([group("claude", [window])]).length, 1);
});

/**
 * The two this rule exists for. Kimi renders its figures only inside its own
 * CLI, and OpenCode publishes spend history but no plan limit — so neither has
 * a number for a limits strip, however much explanation it carries.
 */
test("a runtime with only an explanation is left out", () => {
  const groups = [
    group("claude", [window]),
    group("kimi", [], "Kimi renders its plan usage inside its own CLI"),
    group("opencode", [], "OpenCode publishes no plan limit"),
  ];
  assert.deepEqual(
    stripGroups(groups).map((g) => g.runtime),
    ["claude"],
  );
});

test("nothing readable means nothing to show", () => {
  assert.deepEqual(stripGroups([group("kimi", [], "no")]), []);
  assert.deepEqual(stripGroups([]), []);
});

// Order is how an owner learns where to look; re-sorting would move the bar
// they check first.
test("the surviving runtimes keep their order", () => {
  const groups = [
    group("claude", [window]),
    group("kimi", []),
    group("codex", [window]),
  ];
  assert.deepEqual(
    stripGroups(groups).map((g) => g.runtime),
    ["claude", "codex"],
  );
});
