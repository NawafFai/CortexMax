import * as React from "react";

import { ccPubkeysFrom } from "@/features/messages/lib/ccTags";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { truncateNpub } from "@/shared/lib/pubkey";

/**
 * The reporting line a message was copied to.
 *
 * A copy is delivered and recorded but never starts a turn, so without a
 * visible line it leaves no trace anyone reads — and a record nobody can see
 * protects nobody. This is the trace: who was kept in the loop, shown where
 * the message is, in the order the sender named them.
 */
export function MessageCcLine({ tags }: { tags?: string[][] }) {
  const pubkeys = React.useMemo(() => ccPubkeysFrom(tags), [tags]);
  const profiles = useUsersBatchQuery(pubkeys);

  if (pubkeys.length === 0) return null;

  const names = pubkeys.map((pubkey) => {
    const profile = profiles.data?.profiles?.[pubkey.toLowerCase()];
    // The key itself is a poor name but a true one, and a copy with no
    // legible recipient still tells the reader that a copy was sent.
    return profile?.displayName?.trim() || truncateNpub(pubkey);
  });

  return (
    <div
      className="mt-1 text-2xs text-muted-foreground/70"
      data-testid="message-cc-line"
    >
      <span className="font-medium uppercase tracking-wide">CC</span>{" "}
      <span>{names.join(" · ")}</span>
    </div>
  );
}
