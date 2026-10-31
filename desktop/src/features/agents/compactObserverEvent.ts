/**
 * Keep an agent's work steps small enough to hold in the app's memory.
 *
 * Every step an agent takes reaches the app as an observer event, with its
 * full content: the prompt of each turn, every file it reads or writes, every
 * command and its output. A team working for hours on a real project — 60 KB
 * requirement files, a downloaded ffmpeg — filled the window with a gigabyte
 * of text nobody reads twice. The steps are shown, not replayed: the agents'
 * own files and logs on disk keep everything.
 *
 * Only display material is trimmed. Requests the owner reviews (a new agent's
 * instructions run to 20,000 characters), settings and control results pass
 * through untouched, so nothing that acts on the text is ever handed a cut.
 */

/** Event kinds whose payload is only shown in the activity view. */
const DISPLAY_KINDS = new Set(["acp_read", "acp_write", "raw_json_rpc"]);

/** Longest single text kept from a work step. */
export const MAX_KEPT_TEXT = 16_000;

/** Nesting depth past which a payload is kept as it is, not walked. */
const MAX_DEPTH = 32;

export function truncationMarker(dropped: number): string {
  return `\n… [${dropped.toLocaleString("en-US")} more characters not kept in the app]`;
}

function compactValue(value: unknown, depth: number): unknown {
  if (typeof value === "string") {
    return value.length > MAX_KEPT_TEXT
      ? value.slice(0, MAX_KEPT_TEXT) +
          truncationMarker(value.length - MAX_KEPT_TEXT)
      : value;
  }
  if (value === null || typeof value !== "object" || depth > MAX_DEPTH) {
    return value;
  }
  let changed = false;
  if (Array.isArray(value)) {
    const items = value.map((item) => {
      const next = compactValue(item, depth + 1);
      if (next !== item) changed = true;
      return next;
    });
    return changed ? items : value;
  }
  const fields: Record<string, unknown> = {};
  for (const [key, field] of Object.entries(value)) {
    const next = compactValue(field, depth + 1);
    if (next !== field) changed = true;
    fields[key] = next;
  }
  return changed ? fields : value;
}

/**
 * The event with every oversized text in its payload cut to
 * {@link MAX_KEPT_TEXT}; the same object when nothing needed cutting.
 */
export function compactObserverEvent<
  T extends { kind: string; payload: unknown },
>(event: T): T {
  if (!DISPLAY_KINDS.has(event.kind)) return event;
  const payload = compactValue(event.payload, 0);
  return payload === event.payload ? event : { ...event, payload };
}
