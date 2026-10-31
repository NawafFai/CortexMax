import assert from "node:assert/strict";
import test from "node:test";

import {
  addApprovedCheck,
  projectTeamDraft,
  projectTeamSaveError,
  releaseProjectMember,
  removeApprovedCheck,
  selectProjectCoordinator,
} from "./projectTeamDraft.ts";

function status(overrides = {}) {
  return {
    generation: 4,
    binding_generation: 2,
    coordinator_pubkey: "cortex",
    members: [
      {
        pubkey: "cortex",
        display_name: "CORTEX",
        role: "coordinator",
        capabilities: ["read", "manage_team"],
      },
      {
        pubkey: "byte",
        display_name: "Byte",
        role: "reviewer",
        capabilities: ["read", "review"],
      },
    ],
    available_agents: [
      {
        pubkey: "cortex",
        display_name: "CORTEX",
        runtime_id: "codex",
        local: true,
      },
      {
        pubkey: "byte",
        display_name: "Byte",
        runtime_id: "claude",
        local: true,
      },
      {
        pubkey: "pilot",
        display_name: "Pilot",
        runtime_id: null,
        local: false,
      },
    ],
    paused: false,
    execution_blockers: [],
    ...overrides,
  };
}

test("draft edits do not mutate canonical membership or permissions", () => {
  const original = status();
  const draft = projectTeamDraft(original);
  draft.members[0].capabilities.push("edit");
  draft.members[1].role = "member";
  assert.deepEqual(original.members[0].capabilities, ["read", "manage_team"]);
  assert.equal(original.members[1].role, "reviewer");
  assert.equal(
    projectTeamSaveError(projectTeamDraft(original), original),
    null,
  );
});

test("new coordinator starts read-only without implicit authority", () => {
  const original = status();
  const draft = selectProjectCoordinator(
    projectTeamDraft(original),
    original,
    "pilot",
  );
  assert.equal(draft.coordinator_pubkey, "pilot");
  assert.equal(draft.members[0].role, "member");
  assert.equal(draft.members[1].role, "reviewer");
  assert.deepEqual(draft.members[2].capabilities, ["read"]);
  assert.equal(draft.members[2].role, "coordinator");
  assert.equal(projectTeamSaveError(draft, original), null);
});

test("coordinator change preserves only explicitly existing permissions", () => {
  const original = status();
  const draft = selectProjectCoordinator(
    projectTeamDraft(original),
    original,
    "byte",
  );
  assert.deepEqual(draft.members[1].capabilities, ["read", "review"]);
  assert.equal(
    draft.members.filter((member) => member.role === "coordinator").length,
    1,
  );
  assert.throws(
    () => selectProjectCoordinator(draft, original, "invented"),
    /existing agent/,
  );
});

test("releasing the coordinator clears authority without altering agent inventory", () => {
  const original = status();
  const draft = releaseProjectMember(projectTeamDraft(original), "cortex");
  assert.equal(draft.coordinator_pubkey, null);
  assert.equal(draft.members.length, 1);
  assert.equal(original.available_agents.length, 3);
  assert.equal(original.members.length, 2);
  assert.equal(projectTeamSaveError(draft, original), null);
});

test("stale revisions and disappeared identities require reload", () => {
  const original = status();
  const draft = projectTeamDraft(original);
  assert.match(
    projectTeamSaveError(draft, status({ generation: 5 })),
    /changed/,
  );
  assert.match(
    projectTeamSaveError(draft, status({ binding_generation: 3 })),
    /folder changed/,
  );
  assert.match(
    projectTeamSaveError(
      draft,
      status({ available_agents: original.available_agents.slice(1) }),
    ),
    /no longer available/,
  );
});

test("duplicate identities and mismatched coordinator roles are rejected", () => {
  const original = status();
  const duplicate = projectTeamDraft(original);
  duplicate.members.push(duplicate.members[0]);
  assert.match(projectTeamSaveError(duplicate, original), /only join once/);
  const mismatched = projectTeamDraft(original);
  mismatched.members[1].role = "coordinator";
  assert.match(projectTeamSaveError(mismatched, original), /exactly one/);
  const missing = projectTeamDraft(original);
  missing.coordinator_pubkey = "pilot";
  missing.members[0].role = "member";
  assert.match(projectTeamSaveError(missing, original), /must be a project/);
});

test("approved checks are trimmed, distinct, bounded and validated before save", () => {
  const original = status({ approved_checks: ["cargo test --offline"] });
  const draft = projectTeamDraft(original);
  assert.deepEqual(draft.approved_checks, ["cargo test --offline"]);
  draft.approved_checks.push("mutated");
  assert.deepEqual(original.approved_checks, ["cargo test --offline"]);
  const added = addApprovedCheck(projectTeamDraft(original), "  pnpm test  ");
  assert.deepEqual(added.approved_checks, [
    "cargo test --offline",
    "pnpm test",
  ]);
  assert.equal(
    addApprovedCheck(added, "pnpm test"),
    added,
    "duplicates are ignored",
  );
  assert.throws(() => addApprovedCheck(added, "multi\nline"));
  assert.throws(() => addApprovedCheck(added, "   "));
  const removed = removeApprovedCheck(added, "pnpm test");
  assert.deepEqual(removed.approved_checks, ["cargo test --offline"]);
  assert.equal(projectTeamSaveError(added, original), null);
  const broken = { ...added, approved_checks: ["a", "a"] };
  assert.match(projectTeamSaveError(broken, original), /distinct/);
});

test("a status without the new fields still yields a valid draft", () => {
  const legacy = status();
  delete legacy.approved_checks;
  const draft = projectTeamDraft(legacy);
  assert.deepEqual(draft.approved_checks, []);
  assert.equal(projectTeamSaveError(draft, legacy), null);
});
