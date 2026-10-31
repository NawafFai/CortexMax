import assert from "node:assert/strict";
import { test } from "node:test";

import {
  limitZone,
  remainingPercent,
  budgetFraction,
  forecastExhaustion,
  formatCountdown,
  formatResetInstant,
  budgetZone,
  buildBucketBoundaries,
  channelBudget,
  formatTokens,
  formatUsd,
  localMidnightUnix,
  MIN_TURNS_TO_COMPARE,
  readTokenField,
  rollupByModel,
} from "./usageWindow.ts";

test("boundaries bound N civil days and end with tomorrow's midnight", () => {
  const now = new Date(2026, 8, 19, 14, 30, 0);
  const boundaries = buildBucketBoundaries(7, now);
  assert.equal(boundaries.length, 8, "7 buckets need 8 boundaries");
  assert.equal(
    boundaries.at(-2),
    localMidnightUnix(now),
    "the final bucket starts at today's midnight so today's spend lands in it",
  );
  assert.ok(
    boundaries.at(-1) > localMidnightUnix(now),
    "and ends after it, exclusively",
  );
});

test("boundaries are strictly increasing, which Rust validation requires", () => {
  const boundaries = buildBucketBoundaries(30, new Date(2026, 2, 8, 9, 0, 0));
  for (let i = 0; i + 1 < boundaries.length; i += 1) {
    assert.ok(
      boundaries[i] < boundaries[i + 1],
      `boundary ${i} must precede ${i + 1}`,
    );
  }
});

test("each bucket is one civil day, not a fixed 86400 seconds", () => {
  // Walk a window wide enough to contain a DST transition in zones that have
  // one. Every interval must still be a whole number of hours within the
  // 23-25 hour civil-day range Rust accepts (<= 48h).
  const boundaries = buildBucketBoundaries(30, new Date(2026, 2, 20, 12, 0, 0));
  for (let i = 0; i + 1 < boundaries.length; i += 1) {
    const interval = boundaries[i + 1] - boundaries[i];
    assert.ok(
      interval >= 23 * 3600 && interval <= 25 * 3600,
      `interval ${i} was ${interval}s, outside one civil day`,
    );
  }
});

test("a one-day window is still valid: two boundaries, one bucket", () => {
  const boundaries = buildBucketBoundaries(1, new Date(2026, 8, 19, 1, 0, 0));
  assert.equal(boundaries.length, 2);
});

test("unknown token fields stay null rather than collapsing to zero", () => {
  assert.equal(readTokenField(null), null);
  assert.equal(readTokenField({ value: null }), null);
  assert.equal(readTokenField({ value: "not a number" }), null);
  assert.equal(readTokenField({ value: "90139118" }), 90139118);
});

test("budget zones cross at 70% and 90%", () => {
  assert.equal(budgetZone(10, 50), "ok");
  assert.equal(budgetZone(34.9, 50), "ok");
  assert.equal(budgetZone(35, 50), "warn", "70% is the first yellow point");
  assert.equal(budgetZone(44.9, 50), "warn");
  assert.equal(budgetZone(45, 50), "over", "90% is the first red point");
  assert.equal(budgetZone(153.73, 50), "over");
});

test("no budget means no zone, so the UI shows a figure instead of a bar", () => {
  assert.equal(budgetZone(10, null), null);
  assert.equal(budgetZone(10, 0), null);
  assert.equal(budgetZone(null, 50), null);
});

test("bar fill clamps so an overrun renders full, never past the track", () => {
  assert.equal(budgetFraction(0, 50), 0);
  assert.equal(budgetFraction(25, 50), 0.5);
  assert.equal(budgetFraction(153.73, 50), 1);
  assert.equal(budgetFraction(null, 50), 0);
  assert.equal(budgetFraction(10, 0), 0);
});

test("token and cost formatting", () => {
  assert.equal(formatTokens(null), "—");
  assert.equal(formatTokens(842), "842");
  assert.equal(formatTokens(12_364_175), "12.4M");
  assert.equal(formatTokens(90_139_118), "90.1M");
  assert.equal(formatUsd(null), "—", "unknown cost is not $0.00");
  assert.equal(formatUsd(153.73), "$153.73");
});

// ── per-runtime rollup ──────────────────────────────────────────────────────

import {
  coverageNote,
  rollupByRuntime,
  runtimeBudget,
  runtimeLabel,
} from "./usageWindow.ts";

const tok = (value) => ({ value: value == null ? null : String(value) });
const model = (harness, input, output, cost, turns) => ({
  harness,
  reportCount: turns,
  usage: {
    inputTokens: tok(input),
    outputTokens: tok(output),
    estimatedCostUsd: { value: cost },
  },
});

test("rolls every agent's model rows up by harness", () => {
  const rows = rollupByRuntime([
    { models: [model("claude-agent-acp", 1000, 100, 1.5, 3)] },
    { models: [model("claude-agent-acp", 500, 50, 0.5, 2)] },
    { models: [model("codex-acp", 800, 80, null, 4)] },
  ]);
  const claude = rows.find((r) => r.harness === "claude-agent-acp");
  assert.equal(claude.turns, 5, "turns add across agents");
  assert.equal(claude.inputTokens, 1500);
  assert.equal(claude.costUsd, 2);
  assert.equal(claude.label, "Claude");
});

test("a harness that reports tokens but never a price is not $0.00", () => {
  const rows = rollupByRuntime([
    { models: [model("codex-acp", 800, 80, null, 4)] },
  ]);
  const codex = rows[0];
  assert.equal(codex.coverage, "tokens-only");
  assert.equal(codex.costUsd, null, "unknown price stays null, never zero");
  assert.equal(codex.inputTokens, 800, "but its tokens are still counted");
  assert.match(coverageNote(codex.coverage), /reports no price/);
});

test("rows sort by cost, with priced runtimes above unpriced ones", () => {
  const rows = rollupByRuntime([
    { models: [model("codex-acp", 9_000_000, 900, null, 90)] },
    { models: [model("claude-agent-acp", 1000, 100, 0.25, 1)] },
  ]);
  assert.equal(
    rows[0].harness,
    "claude-agent-acp",
    "a priced runtime ranks first",
  );
  assert.equal(rows[1].harness, "codex-acp");
});

test("a model row with no harness is kept as unknown, never dropped", () => {
  const rows = rollupByRuntime([{ models: [model(null, 700, 70, 0.1, 2)] }]);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].harness, "unknown");
  assert.equal(rows[0].inputTokens, 700, "its tokens still reach the total");
});

test("an agent with no model rows contributes nothing and does not throw", () => {
  assert.deepEqual(rollupByRuntime([{ models: [] }]), []);
  assert.deepEqual(rollupByRuntime([]), []);
});

test("harness ids map to short names, unknown ids pass through", () => {
  assert.equal(runtimeLabel("claude-agent-acp"), "Claude");
  assert.equal(runtimeLabel("codex-acp"), "Codex");
  assert.equal(runtimeLabel("opencode"), "OpenCode");
  assert.equal(runtimeLabel("some-new-harness"), "some-new-harness");
});

test("a runtime budget counts only when set to a positive number", () => {
  assert.equal(runtimeBudget({ "codex-acp": 20 }, "codex-acp"), 20);
  assert.equal(runtimeBudget({ "codex-acp": 0 }, "codex-acp"), null);
  assert.equal(runtimeBudget({}, "codex-acp"), null);
});

test("a countdown shows the precision that matters at each range", () => {
  assert.equal(formatCountdown(2 * 86_400 + 4 * 3_600), "2d 4h");
  assert.equal(formatCountdown(4 * 3_600 + 12 * 60), "4h 12m");
  assert.equal(formatCountdown(12 * 60 + 3), "12m 03s");
  assert.equal(formatCountdown(59), "0m 59s");
});

test("a reset already past reads as due, never as a negative countdown", () => {
  assert.equal(formatCountdown(0), "due now");
  assert.equal(formatCountdown(-500), "due now");
  assert.equal(formatCountdown(Number.NaN), "due now");
});

test("a countdown rolls its units over cleanly", () => {
  assert.equal(formatCountdown(3_600), "1h 0m");
  assert.equal(formatCountdown(3_599), "59m 59s");
  assert.equal(formatCountdown(86_400), "1d 0h");
  assert.equal(formatCountdown(86_399), "23h 59m");
});

test("Kimi's harness ids map to its name, so its spend is not a raw id", () => {
  assert.equal(runtimeLabel("kimi"), "Kimi");
  assert.equal(runtimeLabel("kimi-code"), "Kimi");
});

test("a window burning faster than its length projects an exhaustion time", () => {
  const windowMinutes = 10_080; // a week
  const resetsAt = 1_790_000_000;
  const startedAt = resetsAt - windowMinutes * 60;
  // Half the week gone, 80% of the budget spent: it will not last the week.
  const now = startedAt + windowMinutes * 60 * 0.5;
  const forecast = forecastExhaustion({
    usedPercent: 80,
    resetsAt,
    windowMinutes,
    now,
  });
  assert.ok(forecast, "80% spent at the halfway mark must project");
  // Spent at elapsed/used = 0.5/0.8 = 0.625 of the window.
  assert.equal(forecast.at, startedAt + windowMinutes * 60 * 0.625);
  assert.ok(forecast.inSeconds > 0);
});

test("a window that will comfortably last is not forecast at all", () => {
  const windowMinutes = 10_080;
  const resetsAt = 1_790_000_000;
  const startedAt = resetsAt - windowMinutes * 60;
  // Half the week gone, a third spent: it lasts. A warning here would turn a
  // calm week into an alarm.
  assert.equal(
    forecastExhaustion({
      usedPercent: 33,
      resetsAt,
      windowMinutes,
      now: startedAt + windowMinutes * 60 * 0.5,
    }),
    null,
  );
});

test("too little of the window elapsed means no extrapolation", () => {
  const windowMinutes = 10_080;
  const resetsAt = 1_790_000_000;
  const startedAt = resetsAt - windowMinutes * 60;
  // Ten minutes in, a burst of work would imply the week ends by lunchtime.
  assert.equal(
    forecastExhaustion({
      usedPercent: 5,
      resetsAt,
      windowMinutes,
      now: startedAt + 600,
    }),
    null,
    "a warning that cries wolf on the first busy hour gets ignored",
  );
});

test("a forecast needs both a reset time and a window length", () => {
  const base = {
    usedPercent: 90,
    resetsAt: 1_790_000_000,
    windowMinutes: 10_080,
  };
  const now = base.resetsAt - 10_080 * 60 * 0.5;
  assert.equal(forecastExhaustion({ ...base, resetsAt: null, now }), null);
  assert.equal(forecastExhaustion({ ...base, windowMinutes: null, now }), null);
  assert.equal(forecastExhaustion({ ...base, windowMinutes: 0, now }), null);
});

test("nothing spent, or everything spent, yields no forecast", () => {
  const windowMinutes = 10_080;
  const resetsAt = 1_790_000_000;
  const now = resetsAt - windowMinutes * 60 * 0.5;
  assert.equal(
    forecastExhaustion({ usedPercent: 0, resetsAt, windowMinutes, now }),
    null,
  );
  assert.equal(
    forecastExhaustion({ usedPercent: 100, resetsAt, windowMinutes, now }),
    null,
    "already spent leaves nothing to project",
  );
});

test("a window already past its reset is not forecast", () => {
  const windowMinutes = 10_080;
  const resetsAt = 1_790_000_000;
  assert.equal(
    forecastExhaustion({
      usedPercent: 90,
      resetsAt,
      windowMinutes,
      now: resetsAt + 60,
    }),
    null,
  );
});

/**
 * Per-project budgets. The fallback is the whole point: before these, every
 * channel was scaled against one account-wide figure, so N projects sharing
 * $50 all read green until one of them had spent the day by itself.
 */
test("a project without its own budget follows the account-wide daily figure", () => {
  assert.equal(channelBudget({}, "channel-a", 50), 50);
});

test("a project's own budget overrides the account-wide figure", () => {
  assert.equal(channelBudget({ "channel-a": 12 }, "channel-a", 50), 12);
});

test("one project's budget does not leak to another", () => {
  const budgets = { "channel-a": 12 };
  assert.equal(channelBudget(budgets, "channel-b", 50), 50);
});

// A deliberate zero is a budget, not a missing one — `budgetZone` gives a
// non-positive budget no zones, so the bar disappears instead of silently
// rescaling to the account default. Two readers previously disagreed on this.
test("a deliberate zero is kept, not treated as unset", () => {
  assert.equal(channelBudget({ "channel-a": 0 }, "channel-a", 50), 0);
  assert.equal(budgetZone(3, 0), null, "a zero budget has no zones");
});

test("an unusable stored value falls back rather than scaling by garbage", () => {
  for (const bad of [Number.NaN, -5, Number.POSITIVE_INFINITY]) {
    assert.equal(channelBudget({ "channel-a": bad }, "channel-a", 50), 50);
  }
});

/**
 * Model comparison. The point of the table is ranking models against each
 * other, so the tests are mostly about what must NOT be ranked.
 */
function modelRow(harness, model, turns, cost, output) {
  return {
    harness,
    model,
    reportCount: turns,
    usage: {
      inputTokens: { value: null },
      outputTokens: { value: output == null ? null : String(output) },
      estimatedCostUsd: { value: cost },
    },
  };
}

test("models are ranked by cost per turn, not by total spend", () => {
  // The cheap model has the larger total purely by doing more work.
  const rows = rollupByModel([
    {
      agentPubkey: "a",
      models: [
        modelRow("codex-acp", "cheap", 1000, 100),
        modelRow("claude-agent-acp", "dear", 100, 50),
      ],
    },
  ]);
  assert.equal(rows[0].model, "dear", "0.5/turn must outrank 0.1/turn");
  assert.equal(rows[0].costPerTurn, 0.5);
  assert.equal(rows[1].costPerTurn, 0.1);
});

test("the same model on two runtimes stays two rows", () => {
  const rows = rollupByModel([
    {
      agentPubkey: "a",
      models: [
        modelRow("codex-acp", "shared-name", 30, 3),
        modelRow("opencode", "shared-name", 30, 6),
      ],
    },
  ]);
  assert.equal(rows.length, 2, "a shared model name is not one model");
  assert.deepEqual(rows.map((row) => row.runtimeLabel).sort(), [
    "Codex",
    "OpenCode",
  ]);
});

test("one model's turns are summed across every agent using it", () => {
  const rows = rollupByModel([
    { agentPubkey: "a", models: [modelRow("codex-acp", "m", 15, 3)] },
    { agentPubkey: "b", models: [modelRow("codex-acp", "m", 15, 3)] },
  ]);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].turns, 30);
  assert.equal(rows[0].agents, 2);
  assert.equal(rows[0].costUsd, 6);
});

// A two-turn fluke must never take the headline position, however dear it is.
test("a model under the floor is shown but never ranked above a measured one", () => {
  const rows = rollupByModel([
    {
      agentPubkey: "a",
      models: [
        modelRow("codex-acp", "barely-used", 2, 99),
        modelRow("codex-acp", "measured", 500, 5),
      ],
    },
  ]);
  assert.equal(rows[0].model, "measured");
  assert.equal(rows[0].comparable, true);
  assert.equal(rows[1].model, "barely-used");
  assert.equal(rows[1].comparable, false, "2 turns is not a measurement");
});

test("exactly the floor counts as comparable", () => {
  const rows = rollupByModel([
    {
      agentPubkey: "a",
      models: [modelRow("codex-acp", "m", MIN_TURNS_TO_COMPARE, 1)],
    },
  ]);
  assert.equal(rows[0].comparable, true);
});

test("a harness reporting no price yields turns but no rate", () => {
  const rows = rollupByModel([
    { agentPubkey: "a", models: [modelRow("opencode", "free", 40, null, 800)] },
  ]);
  assert.equal(rows[0].costUsd, null);
  assert.equal(rows[0].costPerTurn, null, "no price means no rate, not zero");
  assert.equal(
    rows[0].outputPerTurn,
    20,
    "tokens still give a per-turn figure",
  );
});

// Folding an unnamed model into a named one would skew that model's average.
test("rows with no model group as unknown rather than merging into a named model", () => {
  const rows = rollupByModel([
    {
      agentPubkey: "a",
      models: [
        modelRow("codex-acp", null, 30, 60),
        modelRow("codex-acp", "named", 30, 3),
      ],
    },
  ]);
  const named = rows.find((row) => row.model === "named");
  assert.equal(named.costPerTurn, 0.1, "the named model keeps its own average");
  assert.ok(rows.some((row) => row.model === "unknown"));
});

test("no agents means no rows, not a row of zeros", () => {
  assert.deepEqual(rollupByModel([]), []);
});

// A provider window reads like a battery: what is left, green while more than
// half remains, yellow from half, red when it is about to run out.
test("a full window is green and reads as all of it left", () => {
  assert.equal(remainingPercent(0), 100);
  assert.equal(limitZone(0), "ok");
  assert.equal(limitZone(8), "ok");
  assert.equal(limitZone(49), "ok");
});

test("half gone turns yellow", () => {
  assert.equal(remainingPercent(50), 50);
  assert.equal(limitZone(50), "warn");
  assert.equal(limitZone(80), "warn");
});

test("almost spent is red, and spent reads as nothing left", () => {
  assert.equal(limitZone(85), "over");
  assert.equal(limitZone(100), "over");
  assert.equal(remainingPercent(100), 0);
  assert.equal(remainingPercent(120), 0, "an overrun is still empty");
  assert.equal(remainingPercent(-5), 100, "a negative reading is still full");
});

test("a reset reads as the day and hour it comes back, in the owner's zone", () => {
  // Wed Sep 30 2026, 11:15 UTC — 2:15 PM in Istanbul, where the owner reads it.
  const at = Date.UTC(2026, 8, 30, 11, 15) / 1000;
  const plain = (text) => text.replace(/\s/g, " ");
  assert.equal(
    plain(formatResetInstant(at, "Europe/Istanbul")),
    "Wed, Sep 30, 2:15 PM",
  );
  assert.equal(plain(formatResetInstant(at, "UTC")), "Wed, Sep 30, 11:15 AM");
});
