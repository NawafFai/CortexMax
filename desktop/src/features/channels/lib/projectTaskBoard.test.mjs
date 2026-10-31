import assert from "node:assert/strict";
import test from "node:test";

import {
  BOARD_COLUMNS,
  describeDispatch,
  describeReadiness,
  groupTasks,
  ownerActionsFor,
  reportLines,
} from "./projectTaskBoard.ts";

function task(state, overrides = {}) {
  return {
    task_id: `t-${state}`,
    channel_id: "c",
    title: state,
    description: "",
    state,
    state_before_pause: null,
    assignee_pubkey: null,
    created_by: "owner",
    required_inputs: [],
    dependencies: [],
    outputs: [],
    return_to: "owner",
    blocker: null,
    review: null,
    report: null,
    revision: 1,
    binding_generation: 1,
    team_generation: 1,
    created_at: 0,
    updated_at: 0,
    dispatch_state: "none",
    dispatch_target: null,
    dispatch_event_id: null,
    dispatch_attempted_at: null,
    ...overrides,
  };
}

test("owner actions never offer acceptance before an approved review state", () => {
  for (const state of [
    "planned",
    "assigned",
    "running",
    "delivered",
    "under_review",
    "blocked",
  ])
    assert.ok(!ownerActionsFor(task(state)).includes("accept"), state);
  assert.deepEqual(ownerActionsFor(task("awaiting_owner_approval")), [
    "accept",
    "pause",
    "cancel",
  ]);
  assert.deepEqual(ownerActionsFor(task("paused")), ["resume", "cancel"]);
  assert.deepEqual(ownerActionsFor(task("completed")), []);
  assert.deepEqual(ownerActionsFor(task("cancelled")), []);
});

test("every task state lands in exactly one board column", () => {
  const states = [
    "planned",
    "waiting_input",
    "assigned",
    "running",
    "delivered",
    "under_review",
    "blocked",
    "awaiting_owner_approval",
    "completed",
    "paused",
    "cancelled",
  ];
  const grouped = groupTasks(states.map((state) => task(state)));
  const seen = grouped.flatMap((column) =>
    column.tasks.map((item) => item.state),
  );
  assert.deepEqual([...seen].sort(), [...states].sort());
  assert.equal(BOARD_COLUMNS.length, grouped.length);
});

test("readiness rungs only authorize execution at the last two states", () => {
  assert.equal(describeReadiness("attached").canStartTask, false);
  assert.equal(describeReadiness("access_verified").canStartTask, false);
  assert.equal(describeReadiness("execution_authorized").canStartTask, true);
  assert.equal(describeReadiness("task_running").canStartTask, true);
});

test("report lines keep English terms on their own left-to-right lines", () => {
  const lines = reportLines(
    "تم تشغيل الفحص التالي\ncargo test --offline\nونجح بالكامل\n",
  );
  assert.deepEqual(lines, [
    { text: "تم تشغيل الفحص التالي", dir: "rtl" },
    { text: "cargo test --offline", dir: "ltr" },
    { text: "ونجح بالكامل", dir: "rtl" },
  ]);
});

test("uncertain dispatch is explained as needing owner reconciliation", () => {
  assert.match(
    describeDispatch(task("assigned", { dispatch_state: "uncertain" })),
    /unknown/,
  );
  assert.match(
    describeDispatch(
      task("assigned", { dispatch_state: "sent", dispatch_event_id: "in-app" }),
    ),
    /board/,
  );
});
