import { mentionPubkeysFrom } from "@/features/messages/lib/ccTags";
import * as React from "react";

import { useKeepMentionedAgentsPinned } from "@/features/messages/lib/autoPinMentionedAgentsPreference";
import {
  initializePersistentAgentAudience,
  usePersistentAgentAudience,
} from "@/features/messages/lib/persistentAgentAudience";

export function useThreadAgentAudience({
  isAgentPubkey,
  rootTags,
  scope,
}: {
  isAgentPubkey: (pubkey: string) => boolean;
  rootTags: readonly string[][];
  scope: string | null;
}) {
  const audience = usePersistentAgentAudience(scope);
  const keepMentionedAgentsPinned = useKeepMentionedAgentsPinned();

  const rootAgentPubkeys = React.useMemo(
    () =>
      // Copies are excluded on purpose: pinning an agent because its
      // manager was kept in the loop would turn a record into an audience.
      mentionPubkeysFrom(rootTags).filter((pubkey) => isAgentPubkey(pubkey)),
    [isAgentPubkey, rootTags],
  );

  React.useEffect(() => {
    if (!scope || !keepMentionedAgentsPinned) return;
    initializePersistentAgentAudience(scope, rootAgentPubkeys);
  }, [keepMentionedAgentsPinned, rootAgentPubkeys, scope]);

  return { audience, keepMentionedAgentsPinned };
}
