import assert from "node:assert/strict";
import test from "node:test";

import {
  FOLDER_PROJECT_REQUEST,
  folderProjectCreatedNote,
  parseFolderProjectRequest,
} from "./folderProjectRequest.ts";

const CHANNEL = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

function payload(request, overrides = {}) {
  return {
    type: FOLDER_PROJECT_REQUEST,
    action: "create",
    requestId: "8d4c0ea4-4b9c-4c55-9a3e-2b1f0a7d9e11",
    request: {
      channelId: CHANNEL,
      name: "demo-app",
      folder: "/Users/me/Desktop/Demo App - v2",
      visibility: "open",
      ...request,
    },
    ...overrides,
  };
}

test("the request the CLI sends is read as sent", () => {
  const parsed = parseFolderProjectRequest(
    payload({ name: "  demo-app  ", description: "  Marketing site " }),
  );
  assert.deepEqual(parsed?.request, {
    channelId: CHANNEL,
    name: "demo-app",
    folder: "/Users/me/Desktop/Demo App - v2",
    visibility: "open",
    description: "Marketing site",
  });
});

test("anything outside the contract refuses the whole request", () => {
  const refused = [
    payload({ root: "/" }),
    payload({ folder: "Desktop/relative" }),
    payload({ folder: "/Users/me/Desktop/evil\nname" }),
    payload({ channelId: "general" }),
    payload({ visibility: "public" }),
    payload({ name: "" }),
    payload({}, { type: "project_channel_request" }),
    payload({}, { action: "delete" }),
    null,
    "folder_project_request",
  ];
  for (const candidate of refused) {
    assert.equal(
      parseFolderProjectRequest(candidate),
      null,
      JSON.stringify(candidate),
    );
  }
});

test("the note back to the agent carries both channel ids and a mention", () => {
  const note = folderProjectCreatedNote({
    agentName: "CORTEX",
    workName: "demo-app",
    workId: "w-1",
    managementName: "demo-app-management",
    managementId: "m-1",
    folder: "/Users/me/Desktop/x",
  });
  assert.match(note, /^@CORTEX /);
  assert.match(note, /#demo-app \(w-1\)/);
  assert.match(note, /#demo-app-management \(m-1\)/);
  assert.match(note, /admin of both/);

  const alone = folderProjectCreatedNote({
    agentName: "CORTEX",
    workName: "demo-app",
    workId: "w-1",
    managementName: null,
    managementId: null,
    folder: "/Users/me/Desktop/x",
  });
  assert.match(alone, /admin of it/);
});
