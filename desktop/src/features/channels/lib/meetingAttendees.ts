import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Who is called into a meeting the owner starts in a channel.
 *
 * A project has two rooms: the work channel, where the whole team works, and
 * its `-management` room, where the owner and the leaders decide. A meeting in
 * either room calls in the management room's agents — the leaders — and not
 * the twenty-odd agents of the work channel, who would all answer every
 * sentence. A channel with no management room starts its meeting empty, as
 * before; a direct message keeps calling in its one agent.
 */

const MANAGEMENT_SUFFIX = "-management";

/** Most agents a meeting starts with: the relay's own limit. */
export const MAX_MEETING_AGENTS = 20;

/**
 * The management room whose agents a meeting in `channel` calls in: the
 * channel itself when it is one, else its `<name>-management` pair.
 */
export function managementRoomFor(
  channel: Pick<Channel, "id" | "name" | "channelType">,
  channels: readonly Pick<Channel, "id" | "name" | "channelType">[],
): Pick<Channel, "id" | "name" | "channelType"> | null {
  if (channel.channelType === "dm") return null;
  if (channel.name.endsWith(MANAGEMENT_SUFFIX)) return channel;
  const pair = `${channel.name}${MANAGEMENT_SUFFIX}`;
  return (
    channels.find(
      (candidate) => candidate.name === pair && candidate.channelType !== "dm",
    ) ?? null
  );
}

/**
 * The agents among the management room's members, the owner left out, each
 * once, in the room's order, at most {@link MAX_MEETING_AGENTS}.
 */
export function meetingAttendeePubkeys(
  memberPubkeys: readonly string[],
  agentPubkeys: ReadonlySet<string>,
  currentPubkey: string | undefined,
): string[] {
  const owner = currentPubkey ? normalizePubkey(currentPubkey) : null;
  const seen = new Set<string>();
  const attendees: string[] = [];
  for (const pubkey of memberPubkeys) {
    const normalized = normalizePubkey(pubkey);
    if (normalized === owner || seen.has(normalized)) continue;
    if (!agentPubkeys.has(normalized)) continue;
    seen.add(normalized);
    attendees.push(normalized);
    if (attendees.length === MAX_MEETING_AGENTS) break;
  }
  return attendees;
}
