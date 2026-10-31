import * as React from "react";

const DONE_STORAGE_KEY = "buzz-home-feed-done.v1";
const UNREAD_STORAGE_KEY = "buzz-home-feed-unread.v1";
/**
 * Items the owner has cleared out of the inbox.
 *
 * Separate from "done" because the two mean different things. A done item is
 * handled and still listed; a dismissed one is gone from the inbox whatever
 * the filters say. Clearing had previously been built out of done plus the
 * unread-only switch, which emptied the list without emptying the inbox — the
 * moment that switch went off, everything came back.
 */
const DISMISSED_STORAGE_KEY = "buzz-home-feed-dismissed.v1";
const MAX_ITEMS = 500;

function doneStorageKey(pubkey: string) {
  return `${DONE_STORAGE_KEY}:${pubkey}`;
}

function unreadStorageKey(pubkey: string) {
  return `${UNREAD_STORAGE_KEY}:${pubkey}`;
}

function dismissedStorageKey(pubkey: string) {
  return `${DISMISSED_STORAGE_KEY}:${pubkey}`;
}

function readStoredIds(key: string): string[] {
  if (typeof window === "undefined") return [];
  const raw = window.localStorage.getItem(key);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed
          .filter((v): v is string => typeof v === "string")
          .slice(-MAX_ITEMS)
      : [];
  } catch {
    return [];
  }
}

function writeStoredIds(key: string, ids: string[]) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(key, JSON.stringify(ids.slice(-MAX_ITEMS)));
}

export function useFeedItemState(pubkey: string | undefined) {
  const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
  const key = doneStorageKey(normalizedPubkey);

  const [doneIds, setDoneIds] = React.useState<string[]>(() =>
    readStoredIds(key),
  );
  const [unreadIds, setUnreadIds] = React.useState<string[]>(() =>
    readStoredIds(unreadStorageKey(normalizedPubkey)),
  );
  const [dismissedIds, setDismissedIds] = React.useState<string[]>(() =>
    readStoredIds(dismissedStorageKey(normalizedPubkey)),
  );
  const [loadedPubkey, setLoadedPubkey] = React.useState(normalizedPubkey);

  React.useEffect(() => {
    setDoneIds(readStoredIds(doneStorageKey(normalizedPubkey)));
    setUnreadIds(readStoredIds(unreadStorageKey(normalizedPubkey)));
    setDismissedIds(readStoredIds(dismissedStorageKey(normalizedPubkey)));
    setLoadedPubkey(normalizedPubkey);
  }, [normalizedPubkey]);

  React.useEffect(() => {
    if (loadedPubkey !== normalizedPubkey) return;
    writeStoredIds(doneStorageKey(normalizedPubkey), doneIds);
  }, [loadedPubkey, normalizedPubkey, doneIds]);

  React.useEffect(() => {
    if (loadedPubkey !== normalizedPubkey) return;
    writeStoredIds(unreadStorageKey(normalizedPubkey), unreadIds);
  }, [loadedPubkey, normalizedPubkey, unreadIds]);

  React.useEffect(() => {
    if (loadedPubkey !== normalizedPubkey) return;
    writeStoredIds(dismissedStorageKey(normalizedPubkey), dismissedIds);
  }, [loadedPubkey, normalizedPubkey, dismissedIds]);

  const doneSet = React.useMemo(() => new Set(doneIds), [doneIds]);
  const unreadSet = React.useMemo(() => new Set(unreadIds), [unreadIds]);
  const dismissedSet = React.useMemo(
    () => new Set(dismissedIds),
    [dismissedIds],
  );

  const markDone = React.useCallback((id: string) => {
    setDoneIds((prev) => (prev.includes(id) ? prev : [...prev, id]));
    setUnreadIds((prev) => prev.filter((v) => v !== id));
  }, []);

  const undoDone = React.useCallback((id: string) => {
    setDoneIds((prev) => prev.filter((v) => v !== id));
  }, []);

  const markUnread = React.useCallback((id: string) => {
    setDoneIds((prev) => prev.filter((v) => v !== id));
    setUnreadIds((prev) => (prev.includes(id) ? prev : [...prev, id]));
  }, []);

  const undoUnread = React.useCallback((id: string) => {
    setUnreadIds((prev) => prev.filter((v) => v !== id));
  }, []);

  /** Clear these items out of the inbox for good. */
  const dismiss = React.useCallback((ids: readonly string[]) => {
    if (ids.length === 0) return;
    setDismissedIds((prev) => {
      const next = new Set(prev);
      for (const id of ids) next.add(id);
      return [...next];
    });
  }, []);

  /**
   * Put a cleared batch back.
   *
   * Clearing is one press over a whole inbox, so it needs a way back that does
   * not depend on remembering what was in there. The caller keeps the exact
   * ids it cleared and hands them straight back.
   */
  const undoDismiss = React.useCallback((ids: readonly string[]) => {
    if (ids.length === 0) return;
    const removing = new Set(ids);
    setDismissedIds((prev) => prev.filter((id) => !removing.has(id)));
  }, []);

  return {
    dismiss,
    dismissedSet,
    doneSet,
    markDone,
    markUnread,
    undoDismiss,
    undoDone,
    undoUnread,
    unreadSet,
  };
}

export type FeedItemState = ReturnType<typeof useFeedItemState>;
