import assert from "node:assert/strict";
import test from "node:test";

import { coalesced } from "./useArchiveAgentMetricsBridge.ts";

function fakeClock() {
  const timers = [];
  return {
    schedule: (run, ms) => {
      const handle = { run, ms, cancelled: false };
      timers.push(handle);
      return handle;
    },
    cancel: (handle) => {
      handle.cancelled = true;
    },
    fire: () => {
      const due = timers.splice(0);
      for (const timer of due) if (!timer.cancelled) timer.run();
    },
    pending: () => timers.filter((timer) => !timer.cancelled).length,
  };
}

test("a burst of metric writes notifies the usage views once", () => {
  const clock = fakeClock();
  let notified = 0;
  const burst = coalesced(
    () => {
      notified += 1;
    },
    5000,
    clock.schedule,
    clock.cancel,
  );
  for (let i = 0; i < 50; i += 1) burst.trigger();
  assert.equal(notified, 0, "nothing until the window closes");
  assert.equal(clock.pending(), 1, "one notification scheduled, not fifty");
  clock.fire();
  assert.equal(notified, 1);
});

test("writes after the window notify again", () => {
  const clock = fakeClock();
  let notified = 0;
  const burst = coalesced(
    () => {
      notified += 1;
    },
    5000,
    clock.schedule,
    clock.cancel,
  );
  burst.trigger();
  clock.fire();
  burst.trigger();
  clock.fire();
  assert.equal(notified, 2);
});

test("leaving cancels a pending notification", () => {
  const clock = fakeClock();
  let notified = 0;
  const burst = coalesced(
    () => {
      notified += 1;
    },
    5000,
    clock.schedule,
    clock.cancel,
  );
  burst.trigger();
  burst.dispose();
  clock.fire();
  assert.equal(notified, 0);
});
