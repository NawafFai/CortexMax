import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { FolderProjectRequestDetails } from "./FolderProjectRequestDialog.tsx";

const request = {
  channelId: "7c07e659-3610-42f4-9a5e-1e9973c09da9",
  name: "demo-app",
  folder: "/Users/me/Desktop/Demo App - v2",
  visibility: "open",
};

function render(folderCheck, overrides = {}) {
  return renderToStaticMarkup(
    React.createElement(FolderProjectRequestDetails, {
      request: { ...request, ...overrides },
      folderCheck,
    }),
  );
}

test("the owner sees both rooms and the exact folder before creating", () => {
  const html = render({ status: "ok", root: request.folder });
  assert.match(html, /#demo-app</);
  assert.match(html, /#demo-app-management</);
  assert.match(html, /Demo App - v2/);
  assert.match(html, /read and write access inside it/);
});

test("a refused folder says why, in place of the approval line", () => {
  const html = render({
    status: "refused",
    reason: "an agent cannot propose a folder under ~/Library",
  });
  assert.match(
    html,
    /Not allowed: an agent cannot propose a folder under ~\/Library/,
  );
  assert.doesNotMatch(html, /read and write access/);
});

test("the folder is shown while it is still being checked", () => {
  const html = render({ status: "checking" });
  assert.match(html, /Checking the folder/);
});
