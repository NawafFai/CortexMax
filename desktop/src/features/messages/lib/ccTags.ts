/**
 * Carbon copies on a message.
 *
 * A copy rides a `p` tag so the relay delivers it to the person named, and
 * carries the NIP-10 marker `cc` in the fourth position so a reader can tell
 * it apart from a mention. The harness treats a marked tag as not satisfying
 * a mention requirement: being kept in the loop is not being asked to act.
 *
 * Everything here exists because that distinction has to hold on this side
 * too. A copy counted as a mention would show the wrong audience, pin the
 * wrong agents, and turn "your manager saw this" into "your manager was
 * asked to do something".
 */

const CC_MARKER = "cc";

function isPubkeyTag(tag: readonly string[]): boolean {
  return tag[0] === "p" && typeof tag[1] === "string" && tag[1].length > 0;
}

/** Is this `p` tag a copy rather than an ask? */
export function isCcTag(tag: readonly string[]): boolean {
  return isPubkeyTag(tag) && tag[3] === CC_MARKER;
}

/** The people asked to act — every `p` tag that is not a copy. */
export function mentionPubkeysFrom(
  tags: readonly (readonly string[])[] | undefined,
): string[] {
  return (tags ?? [])
    .filter((tag) => isPubkeyTag(tag) && !isCcTag(tag))
    .map((tag) => tag[1] as string);
}

/** The people copied in — kept in the loop, not asked. */
export function ccPubkeysFrom(
  tags: readonly (readonly string[])[] | undefined,
): string[] {
  const seen = new Set<string>();
  const copies: string[] = [];
  for (const tag of tags ?? []) {
    if (!isCcTag(tag)) continue;
    const pubkey = tag[1] as string;
    const key = pubkey.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    copies.push(pubkey);
  }
  return copies;
}
