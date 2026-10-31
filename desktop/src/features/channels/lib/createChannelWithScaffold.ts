import { toast } from "sonner";

import { setChannelConversationMode } from "@/shared/api/channelConversations";
import { createChannel, ensureChannelHandover } from "@/shared/api/tauri";
import type { Channel, CreateChannelInput } from "@/shared/api/types";
import { handoverTemplate } from "./handoverTemplate";
import {
  managementChannelInput,
  wantsManagementChannel,
} from "./projectScaffold";

export type CreatedChannels = {
  channel: Channel;
  /** The management room, when one was made and it succeeded. */
  management: Channel | null;
};

/**
 * Create a channel; for a project, create its management room too.
 *
 * Everything in one channel means the owner reads every specialist's working
 * chatter to find the lines that were for him. The pair is made here rather
 * than offered afterwards because a room nobody remembers to create is a room
 * that does not exist.
 *
 * The work channel is returned whatever happens to the second one. A
 * management room that failed to appear is worth a toast, not the loss of the
 * project channel that was actually asked for — and the owner can make it by
 * hand. Failing the whole creation over the lesser half would be the wrong
 * trade.
 *
 * One path for every way a project starts — the owner's own create dialog and
 * a project an agent asked for — so the two can never drift into making
 * different projects.
 */
export async function createChannelWithScaffold(
  input: CreateChannelInput,
  scope: { relayUrl?: string; ownerPubkey?: string },
): Promise<CreatedChannels> {
  const created = await createChannel(input);
  let management: Channel | null = null;
  if (wantsManagementChannel(input)) {
    try {
      management = await createChannel(managementChannelInput(input));
      // A management room exists to be talked in. Leaving it mention-only
      // means the owner writes to his own reporting line and nobody answers,
      // which is the failure the room was created to avoid.
      //
      // Silent on failure: the room is the deliverable, and a toast about a
      // setting the owner never asked for would land on top of the one that
      // matters.
      try {
        if (!scope.relayUrl || !scope.ownerPubkey) {
          throw new Error("no identity yet");
        }
        await setChannelConversationMode(
          { ownerPubkey: scope.ownerPubkey, relayUrl: scope.relayUrl },
          management.id,
          true,
        );
      } catch {
        // The menu entry on the channel can still turn it on by hand.
      }
    } catch (cause) {
      toast.warning(
        `Created #${input.name}, but its management channel could not be created: ${
          cause instanceof Error ? cause.message : String(cause)
        }`,
      );
    }
    // Seed the handover with the project rather than waiting for someone to
    // ask for one. The team it serves is the team that has not met yet — the
    // note has to be there before the first agent looks for it, or the first
    // agent to need it is the one who discovers it is missing.
    //
    // Failures are swallowed on purpose: the folder binding may not have
    // landed yet, and a missing note is something the GM can add later. It is
    // never worth losing a created project over.
    try {
      await ensureChannelHandover(
        created.id,
        handoverTemplate(
          input.name.trim(),
          new Date().toISOString().slice(0, 10),
        ),
      );
    } catch {
      // Reported nowhere by design: the owner did not ask for this, and a
      // toast about a note he never requested is noise at the moment he is
      // trying to start work.
    }
  }
  return { channel: created, management };
}
