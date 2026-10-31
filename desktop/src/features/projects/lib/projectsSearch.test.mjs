import assert from "node:assert/strict";
import test from "node:test";

import { matchesProjectsSearch } from "./projectsSearch.ts";

test("matches every case-insensitive token across fields", () => {
  assert.equal(
    matchesProjectsSearch("cortex mobile", [
      "Cortex Max Platform",
      "Desktop, relay, and mobile clients",
    ]),
    true,
  );
  assert.equal(
    matchesProjectsSearch("cortex missing", [
      "Cortex Max Platform",
      "Desktop, relay, and mobile clients",
    ]),
    false,
  );
});

test("empty search matches everything", () => {
  assert.equal(matchesProjectsSearch("   ", []), true);
});
