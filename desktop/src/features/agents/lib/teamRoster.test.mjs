import assert from "node:assert/strict";
import { test } from "node:test";

import {
  formatAgo,
  rollupTeams,
  runtimeTeamLabel,
  teamShare,
} from "./teamRoster.ts";

test("harness ids map to the names the runtimes are called", () => {
  assert.equal(runtimeTeamLabel("claude"), "Claude");
  assert.equal(runtimeTeamLabel("claude-agent-acp"), "Claude");
  assert.equal(runtimeTeamLabel("codex"), "Codex");
  assert.equal(runtimeTeamLabel("codex-acp"), "Codex");
  assert.equal(runtimeTeamLabel("opencode"), "OpenCode");
});

test("an unknown harness id passes through rather than vanishing", () => {
  assert.equal(runtimeTeamLabel("some-new-harness"), "some-new-harness");
});

test("an agent with no runtime is named, not blanked", () => {
  assert.equal(runtimeTeamLabel(null), "No runtime set");
});

test("each runtime's team is counted", () => {
  const rows = rollupTeams([
    { runtime: "claude" },
    { runtime: "claude" },
    { runtime: "codex" },
  ]);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].label, "Claude");
  assert.equal(rows[0].total, 2);
  assert.equal(rows[1].label, "Codex");
  assert.equal(rows[1].total, 1);
});

test("two ids for one runtime fold into a single team", () => {
  const rows = rollupTeams([
    { runtime: "claude" },
    { runtime: "claude-agent-acp" },
  ]);
  assert.equal(rows.length, 2, "distinct harness ids stay distinct rows");
  assert.equal(rows[0].label, "Claude");
  assert.equal(rows[1].label, "Claude");
});

test("running agents are counted separately from the total", () => {
  const rows = rollupTeams([
    { runtime: "codex", status: "running" },
    { runtime: "codex", status: "stopped" },
    { runtime: "codex", status: "ready" },
  ]);
  assert.equal(rows[0].total, 3);
  assert.equal(rows[0].running, 2, "running and ready are both up");
});

test("an agent with no status is not assumed to be running", () => {
  const rows = rollupTeams([{ runtime: "codex" }, { runtime: "codex" }]);
  assert.equal(rows[0].total, 2);
  assert.equal(rows[0].running, 0, "unknown is not running");
});

test("agents with no runtime keep their own row rather than being dropped", () => {
  const rows = rollupTeams([
    { runtime: "claude" },
    { runtime: null },
    { runtime: null },
  ]);
  const total = rows.reduce((sum, row) => sum + row.total, 0);
  assert.equal(total, 3, "the roster must add up to the team the owner has");
  assert.equal(rows[0].label, "No runtime set");
  assert.equal(rows[0].total, 2);
});

test("the largest team sorts first, ties broken by name", () => {
  const rows = rollupTeams([
    { runtime: "opencode" },
    { runtime: "codex" },
    { runtime: "codex" },
    { runtime: "claude" },
  ]);
  assert.deepEqual(
    rows.map((row) => row.label),
    ["Codex", "Claude", "OpenCode"],
  );
});

test("an empty team produces no rows rather than a zero row", () => {
  assert.deepEqual(rollupTeams([]), []);
});

test("a team's share of the whole is a fraction, and never divides by zero", () => {
  const [row] = rollupTeams([{ runtime: "codex" }, { runtime: "codex" }]);
  assert.equal(teamShare(row, 4), 0.5);
  assert.equal(teamShare(row, 0), 0, "no agents means no share, not Infinity");
});

test("an age is described in the unit that matters at its range", () => {
  assert.equal(formatAgo(30), "just now");
  assert.equal(formatAgo(59), "just now");
  assert.equal(formatAgo(60), "1m ago");
  assert.equal(formatAgo(45 * 60), "45m ago");
  assert.equal(formatAgo(3 * 3600), "3h ago");
  assert.equal(formatAgo(4 * 86400), "4d ago");
});

test("an age rolls its units over cleanly", () => {
  assert.equal(formatAgo(3599), "59m ago");
  assert.equal(formatAgo(3600), "1h ago");
  assert.equal(formatAgo(86399), "23h ago");
  assert.equal(formatAgo(86400), "1d ago");
});

test("a clock that went backwards reads as just now, never as a negative age", () => {
  assert.equal(formatAgo(-500), "just now");
  assert.equal(formatAgo(Number.NaN), "unknown");
});
