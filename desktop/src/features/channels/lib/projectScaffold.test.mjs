import assert from "node:assert/strict";
import { test } from "node:test";

import {
  isManagementChannel,
  managementChannelInput,
  managementChannelNameFor,
  wantsManagementChannel,
  workChannelNameFor,
  MANAGEMENT_SUFFIX,
} from "./projectScaffold.ts";

function work(overrides = {}) {
  return {
    name: "product-launch",
    channelType: "stream",
    visibility: "private",
    localProject: { root: "/Users/me/project" },
    ...overrides,
  };
}

test("a project channel gets a management room named after it", () => {
  assert.equal(
    managementChannelNameFor("product-launch"),
    "product-launch-management",
  );
});

test("the pair can be recognised again from either side", () => {
  const name = managementChannelNameFor("alpha");
  assert.ok(isManagementChannel(name));
  assert.equal(workChannelNameFor(name), "alpha");
  assert.equal(
    workChannelNameFor("alpha"),
    null,
    "a work channel has no work channel",
  );
});

/**
 * The suffix is how the pair is recognised, so it must survive the length
 * limit intact — truncating it would produce a channel nothing could match
 * back to its project.
 */
test("a long name is trimmed so the suffix survives whole", () => {
  const name = managementChannelNameFor("x".repeat(200));
  assert.ok(name.length <= 64);
  assert.ok(name.endsWith(MANAGEMENT_SUFFIX));
  assert.ok(isManagementChannel(name));
});

test("a trailing dash is not doubled into the suffix", () => {
  assert.equal(managementChannelNameFor("alpha-"), "alpha-management");
  assert.equal(managementChannelNameFor("  alpha  "), "alpha-management");
});

/**
 * One project per channel is the app's rule. A second channel claiming the
 * same folder would either be refused outright or split ownership of it.
 */
test("the management room never carries the project binding", () => {
  const made = managementChannelInput(work());
  assert.equal(made.localProject, undefined);
  assert.equal(made.name, "product-launch-management");
});

test("it inherits visibility rather than forcing one", () => {
  assert.equal(
    managementChannelInput(work({ visibility: "open" })).visibility,
    "open",
  );
  assert.equal(
    managementChannelInput(work({ visibility: "private" })).visibility,
    "private",
  );
});

test("its description names the channel it manages", () => {
  assert.match(
    managementChannelInput(work()).description,
    /#product-launch/,
  );
});

// A folder is what makes a project; without one there is nothing to manage.
test("a channel with no project folder gets no management room", () => {
  assert.equal(wantsManagementChannel(work()), true);
  assert.equal(
    wantsManagementChannel(work({ localProject: undefined })),
    false,
  );
});

// Otherwise an owner who names a channel "-management" gets a
// "-management-management" beside it.
test("a management room is never given a management room", () => {
  assert.equal(
    wantsManagementChannel(work({ name: "alpha-management" })),
    false,
  );
});
