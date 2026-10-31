import assert from "node:assert/strict";
import test from "node:test";

import { findProjectPairs, planProjectSections } from "./projectSections.ts";

function channel(id, name, extra = {}) {
  return { id, name, channelType: "stream", archivedAt: null, ...extra };
}

const CHANNELS = [
  channel("w1", "product-launch"),
  channel("m1", "product-launch-management"),
  channel("w2", "demo-app"),
  channel("m2", "demo-app-management"),
  channel("g", "general"),
];

test("a project is a live work channel and its management room", () => {
  assert.deepEqual(findProjectPairs(CHANNELS), [
    { name: "product-launch", workId: "w1", managementId: "m1" },
    { name: "demo-app", workId: "w2", managementId: "m2" },
  ]);
});

test("half a pair, an archived half, or a DM is not a project", () => {
  assert.deepEqual(
    findProjectPairs([
      channel("m", "orphan-management"),
      channel("w", "old"),
      channel("wm", "old-management", { archivedAt: "2026-09-01" }),
      channel("d", "dm-management", { channelType: "dm" }),
      channel("dw", "dm", { channelType: "dm" }),
    ]),
    [],
  );
});

test("an unarranged project gets a folder named after it with both rooms", () => {
  const plan = planProjectSections(
    findProjectPairs(CHANNELS),
    [],
    {},
    new Set(),
  );
  assert.deepEqual(plan.create, [
    { name: "product-launch", channelIds: ["w1", "m1"] },
    { name: "demo-app", channelIds: ["w2", "m2"] },
  ]);
  assert.deepEqual(plan.assign, []);
  assert.deepEqual(plan.settled, [], "settled only once the folder is seen");
});

test("a folder that already has the project's name is used, not duplicated", () => {
  const plan = planProjectSections(
    findProjectPairs(CHANNELS).slice(0, 1),
    [{ id: "s1", name: "product-launch" }],
    {},
    new Set(),
  );
  assert.deepEqual(plan.create, []);
  assert.deepEqual(plan.assign, [
    { channelId: "w1", sectionId: "s1" },
    { channelId: "m1", sectionId: "s1" },
  ]);
});

test("once both rooms are in the folder the project is settled", () => {
  const plan = planProjectSections(
    findProjectPairs(CHANNELS).slice(0, 1),
    [{ id: "s1", name: "product-launch" }],
    { w1: "s1", m1: "s1" },
    new Set(),
  );
  assert.deepEqual(plan, { create: [], assign: [], settled: ["w1"] });
});

test("half the pair in its folder brings the other half in", () => {
  const plan = planProjectSections(
    findProjectPairs(CHANNELS).slice(0, 1),
    [{ id: "s1", name: "product-launch" }],
    { w1: "s1" },
    new Set(),
  );
  assert.deepEqual(plan.assign, [{ channelId: "m1", sectionId: "s1" }]);
  assert.deepEqual(plan.settled, []);
});

test("the owner's own arrangement is never overridden", () => {
  const plan = planProjectSections(
    findProjectPairs(CHANNELS).slice(0, 1),
    [
      { id: "s1", name: "product-launch" },
      { id: "mine", name: "Research" },
    ],
    { w1: "s1", m1: "mine" },
    new Set(),
  );
  assert.deepEqual(plan, { create: [], assign: [], settled: ["w1"] });
});

test("a settled project is left alone even after its rooms are moved out", () => {
  const plan = planProjectSections(
    findProjectPairs(CHANNELS),
    [],
    {},
    new Set(["w1", "w2"]),
  );
  assert.deepEqual(plan, { create: [], assign: [], settled: [] });
});
