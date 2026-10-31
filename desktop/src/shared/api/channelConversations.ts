import { invokeTauri } from "./tauri";

/**
 * Channels the owner can simply talk in.
 *
 * An agent answers when it is `@`-mentioned, which is right for a busy
 * project room — one untargeted message there would otherwise cost a turn
 * from every agent in it — and wrong for a small one, where the owner types
 * "good job everyone" and nothing happens because nobody was addressed.
 *
 * In a channel listed here the mention requirement is lifted for the owner's
 * own messages only. Agents still tag each other: `owner-only` access admits
 * verified same-owner siblings, so opening a room to everyone would have each
 * reply wake every other agent in it, without end.
 *
 * The setting reaches an agent when it next starts — a relay subscription is
 * fixed at launch — so a caller that changes it should say so.
 */
export type ConversationScope = {
  relayUrl: string;
  ownerPubkey: string;
};

export function listConversationChannels(scope: ConversationScope) {
  return invokeTauri<string[]>("list_conversation_channels", scope);
}

export function setChannelConversationMode(
  scope: ConversationScope,
  channelId: string,
  enabled: boolean,
) {
  return invokeTauri<string[]>("set_channel_conversation_mode", {
    ...scope,
    channelId,
    enabled,
  });
}
