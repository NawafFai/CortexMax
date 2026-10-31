import assert from "node:assert/strict";
import { after, test } from "node:test";
import { JSDOM } from "jsdom";

import { useFeedItemState } from "./useFeedItemState.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  IS_REACT_ACT_ENVIRONMENT: true,
});
after(() => dom.window.close());

const OWNER = "a".repeat(64);

/** Mount the hook. `fresh` wipes stored state so tests cannot leak into
 *  each other through the localStorage this hook persists to. */
async function mount({ fresh = true } = {}) {
  const { renderHook, act, cleanup } = await import("@testing-library/react");
  if (fresh) window.localStorage.clear();
  const { result } = renderHook(() => useFeedItemState(OWNER));
  return { result, act, cleanup };
}

/**
 * The distinction this whole set exists for: "done" leaves an item listed,
 * "dismissed" takes it out of the inbox. Clearing used to be built from done
 * plus the unread-only switch, which emptied the list without emptying the
 * inbox — everything came back the moment that switch went off.
 */
test("a dismissed item is not merely done", async () => {
  const { result, act, cleanup } = await mount();
  await act(async () => result.current.dismiss(["a", "b"]));

  assert.ok(result.current.dismissedSet.has("a"));
  assert.ok(result.current.dismissedSet.has("b"));
  assert.equal(result.current.doneSet.has("a"), false, "clearing is not done");
  cleanup();
});

test("undo restores exactly the batch it was given", async () => {
  const { result, act, cleanup } = await mount();
  await act(async () => result.current.dismiss(["a", "b", "c"]));
  await act(async () => result.current.undoDismiss(["a", "c"]));

  assert.equal(result.current.dismissedSet.has("a"), false);
  assert.equal(result.current.dismissedSet.has("c"), false);
  assert.ok(result.current.dismissedSet.has("b"), "b was not in the undo");
  cleanup();
});

// Clearing twice must not resurrect anything or double-count it.
test("dismissing an already-dismissed id is a no-op", async () => {
  const { result, act, cleanup } = await mount();
  await act(async () => result.current.dismiss(["a"]));
  await act(async () => result.current.dismiss(["a", "b"]));

  assert.equal(result.current.dismissedSet.size, 2);
  cleanup();
});

test("an empty clear changes nothing", async () => {
  const { result, act, cleanup } = await mount();
  await act(async () => result.current.dismiss([]));
  assert.equal(result.current.dismissedSet.size, 0);
  cleanup();
});

/**
 * The point of clearing: it has to still be cleared next time the app opens.
 * A cleared inbox that refills on restart is the same failure as one that
 * refills when a filter changes.
 */
test("a cleared inbox is still cleared after a remount", async () => {
  const first = await mount();
  await first.act(async () => first.result.current.dismiss(["a", "b"]));
  first.cleanup();

  const second = await mount({ fresh: false });
  assert.ok(second.result.current.dismissedSet.has("a"));
  assert.ok(second.result.current.dismissedSet.has("b"));
  second.cleanup();
});

// Two identities on one machine must not clear each other's inboxes.
test("dismissals are scoped to the identity that made them", async () => {
  const { renderHook, act, cleanup } = await import("@testing-library/react");
  window.localStorage.clear();
  const mine = renderHook(() => useFeedItemState(OWNER));
  await act(async () => mine.result.current.dismiss(["a"]));
  cleanup();

  const theirs = renderHook(() => useFeedItemState("b".repeat(64)));
  assert.equal(theirs.result.current.dismissedSet.has("a"), false);
  cleanup();
});
