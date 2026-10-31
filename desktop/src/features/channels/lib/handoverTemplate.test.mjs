import assert from "node:assert/strict";
import { test } from "node:test";

import { HANDOVER_FILENAME, handoverTemplate } from "./handoverTemplate.ts";

test("the handover names the channel and its date", () => {
  const out = handoverTemplate("product-launch", "2026-09-20");
  assert.match(out, /# Handover — product-launch/);
  assert.match(out, /Last updated: 2026-09-20/);
});

/**
 * These four sections are the whole point. A handover that drops "do not
 * build on this yet" is how an unverified increment gets built upon — which
 * is the exact failure this project's own HANDOVER.md was written to stop.
 */
test("it carries the four sections a team actually needs", () => {
  const out = handoverTemplate("x", "2026-01-01");
  for (const heading of [
    "## State of play",
    "## In flight",
    "## Decisions the owner still holds",
    "## Do not build on this yet",
  ]) {
    assert.ok(out.includes(heading), `missing ${heading}`);
  }
});

test("empty sections say they are empty rather than looking unwritten", () => {
  const out = handoverTemplate("x", "2026-01-01");
  assert.match(out, /_\(nothing recorded yet\)_/);
  assert.match(out, /_\(none\)_/);
});

test("the filename is the one the team already uses", () => {
  assert.equal(HANDOVER_FILENAME, "HANDOVER.md");
});
