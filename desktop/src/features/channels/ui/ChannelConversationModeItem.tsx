import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { MessagesSquare } from "lucide-react";
import { toast } from "sonner";

import { useCommunities } from "@/features/communities/useCommunities";
import {
  listConversationChannels,
  setChannelConversationMode,
} from "@/shared/api/channelConversations";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";
import { InfoFieldRow } from "./ChannelManagementSheetRows";

/**
 * Menu entry for "talk here without tagging anyone".
 *
 * Agents answer when `@`-mentioned. In a busy project room that is what keeps
 * one message from costing a turn from every agent in it; in a small room it
 * means the owner types "good job everyone" and nothing happens, because
 * nobody was addressed by name.
 *
 * Turning this on lifts the mention requirement for the owner's own messages
 * in this channel. Agents still tag each other — `owner-only` access admits
 * verified same-owner siblings, so opening a room to everyone would have each
 * reply wake every other agent in it, without end.
 */
export function useChannelConversationMode(channelId: string, owner?: string) {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl;
  const client = useQueryClient();
  const queryKey = ["channel-conversations", relayUrl, owner];
  const enabled = !!relayUrl && !!owner;

  const query = useQuery({
    queryKey,
    enabled,
    queryFn: () =>
      listConversationChannels({
        ownerPubkey: owner as string,
        relayUrl: relayUrl as string,
      }),
    staleTime: 30_000,
  });

  const mutation = useMutation({
    mutationFn: (next: boolean) =>
      setChannelConversationMode(
        { ownerPubkey: owner as string, relayUrl: relayUrl as string },
        channelId,
        next,
      ),
    onSuccess: (channels, next) => {
      client.setQueryData(queryKey, channels);
      toast.success(
        next
          ? "Agents here answer you without a mention. Restart them to apply."
          : "Agents here need a mention again. Restart them to apply.",
      );
    },
    onError: (cause: unknown) =>
      toast.error(cause instanceof Error ? cause.message : String(cause)),
  });

  const isOpen = (query.data ?? []).includes(channelId);

  return {
    /** False without a community and an identity: there is nothing to scope to. */
    available: enabled,
    isOpen,
    pending: mutation.isPending,
    toggle: () => mutation.mutate(!isOpen),
  };
}

/** Menu form, for the layouts that show the channel overflow dropdown. */
export function ChannelConversationModeItem({
  channelId,
  owner,
}: {
  channelId: string;
  owner?: string;
}) {
  const mode = useChannelConversationMode(channelId, owner);
  // A row that cannot act is worse than no row.
  if (!mode.available) return null;

  return (
    <DropdownMenuItem
      data-testid="channel-conversation-mode-trigger"
      disabled={mode.pending}
      onSelect={(event) => {
        // Keep the menu open: the toast explains that a restart is needed,
        // and a row that vanishes as it is pressed hides the result.
        event.preventDefault();
        mode.toggle();
      }}
    >
      <MessagesSquare />
      <span>Talk without @</span>
      <span className="ml-auto text-xs text-muted-foreground">
        {mode.isOpen ? "On" : "Off"}
      </span>
    </DropdownMenuItem>
  );
}

/** Settings-panel form, for the layouts that show Channel Settings instead. */
export function ChannelConversationModeRow({
  channelId,
  owner,
}: {
  channelId: string;
  owner?: string;
}) {
  const mode = useChannelConversationMode(channelId, owner);
  if (!mode.available) return null;

  return (
    <InfoFieldRow
      icon={MessagesSquare}
      label="Talk without @"
      multiline
      onClick={mode.pending ? undefined : mode.toggle}
      testId="channel-conversation-mode-row"
      value={
        mode.isOpen
          ? "On — agents answer you here even when you tag nobody. They still tag each other."
          : "Off — agents here only answer when you @mention them."
      }
    />
  );
}
