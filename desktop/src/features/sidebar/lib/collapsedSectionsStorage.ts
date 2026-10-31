import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

/**
 * Which custom sections the owner folded, kept across restarts.
 *
 * Folding was held in memory only, so every launch opened every section again
 * — tolerable for two sections, not for a sidebar where each project is its
 * own folder and folding the quiet ones is the point.
 */
const KEY_PREFIX = "cortex-max.collapsed-sections.v1";

export function collapsedSectionsKey(
  pubkey: string | undefined,
  relayUrl: string | undefined,
): string | null {
  if (!pubkey || !relayUrl) return null;
  return `${KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

export function readCollapsedSections(
  key: string | null,
): Record<string, boolean> {
  if (key == null) return {};
  try {
    const parsed: unknown = JSON.parse(
      window.localStorage.getItem(key) ?? "{}",
    );
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return {};
    }
    return Object.fromEntries(
      Object.entries(parsed).filter(
        (entry): entry is [string, boolean] => entry[1] === true,
      ),
    );
  } catch {
    return {};
  }
}

export function writeCollapsedSections(
  key: string | null,
  collapsed: Record<string, boolean>,
) {
  if (key == null) return;
  try {
    // Only folded sections are worth a byte; open is the default.
    const folded = Object.fromEntries(
      Object.entries(collapsed).filter(([, value]) => value),
    );
    window.localStorage.setItem(key, JSON.stringify(folded));
  } catch {
    // Storage unavailable: folding still works for this session.
  }
}
