import { strict as assert } from "node:assert";
import test from "node:test";

import {
  MAX_MEETING_AGENTS,
  managementRoomFor,
  meetingAttendeePubkeys,
} from "./meetingAttendees.ts";

const OWNER = "a".repeat(64);
const CORTEX = "b".repeat(64);
const AXEL = "c".repeat(64);
const HUMAN = "d".repeat(64);

const work = { id: "w", name: "product-launch", channelType: "stream" };
const management = {
  id: "m",
  name: "product-launch-management",
  channelType: "stream",
};
const general = { id: "g", name: "general", channelType: "stream" };
const dm = { id: "d", name: "dm", channelType: "dm" };
const channels = [general, work, management, dm];

test("a meeting in the work channel calls in its management room", () => {
  assert.equal(managementRoomFor(work, channels), management);
});

test("a meeting in the management room calls in that room", () => {
  assert.equal(managementRoomFor(management, channels), management);
});

test("a channel with no management room, or a direct message, has none", () => {
  assert.equal(managementRoomFor(general, channels), null);
  assert.equal(managementRoomFor(dm, channels), null);
});

test("only agents are called in, once each, never the owner", () => {
  const agents = new Set([CORTEX, AXEL, OWNER]);
  assert.deepEqual(
    meetingAttendeePubkeys(
      [OWNER, CORTEX, HUMAN, AXEL, CORTEX.toUpperCase()],
      agents,
      OWNER,
    ),
    [CORTEX, AXEL],
  );
});

test("a crowded room is capped at the relay's limit", () => {
  const many = Array.from({ length: 30 }, (_, i) =>
    i.toString(16).padStart(64, "0"),
  );
  const attendees = meetingAttendeePubkeys(many, new Set(many), OWNER);
  assert.equal(attendees.length, MAX_MEETING_AGENTS);
  assert.deepEqual(attendees, many.slice(0, MAX_MEETING_AGENTS));
});
