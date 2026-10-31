import type { CreateChannelInput } from "@/shared/api/types";

/**
 * A project needs two rooms, not one.
 *
 * Everything in a single channel means the owner reads every specialist's
 * working chatter to find the three lines that were for him, and the team
 * writes its status updates in front of an audience it is also reporting to.
 * Splitting them gives each conversation its own room: the work happens in
 * one, and what reaches the owner happens in the other.
 *
 * The management room is created alongside the work room rather than offered
 * afterwards, because a room nobody remembers to make is a room that does not
 * exist — the first project ran for hours in one channel for exactly that
 * reason.
 */

/** Suffix that marks the management room, and how it is recognised again. */
export const MANAGEMENT_SUFFIX = "-management";

/** Is this the management half of a pair? */
export function isManagementChannel(name: string): boolean {
  return name.endsWith(MANAGEMENT_SUFFIX);
}

/** The work channel's name for a given management channel, or null. */
export function workChannelNameFor(managementName: string): string | null {
  if (!isManagementChannel(managementName)) return null;
  return managementName.slice(0, -MANAGEMENT_SUFFIX.length) || null;
}

/**
 * Relay channel names are slug-like, so the pair has to survive the same
 * rules the work channel was already subject to. Trimming to the limit before
 * appending keeps the suffix intact rather than truncating it into something
 * `isManagementChannel` would no longer recognise.
 */
const MAX_NAME = 64;

export function managementChannelNameFor(workName: string): string {
  const base = workName.trim().replace(/-+$/, "");
  const room = MAX_NAME - MANAGEMENT_SUFFIX.length;
  return `${base.slice(0, room)}${MANAGEMENT_SUFFIX}`;
}

/**
 * Build the management channel's creation input from the work channel's.
 *
 * It deliberately does NOT carry the local project binding. One project per
 * channel is the app's own rule, and the work is bound to the work room; a
 * second channel claiming the same folder would either be refused or would
 * split ownership of it. The management room is for talking about the work,
 * so it needs no folder of its own.
 *
 * Visibility is inherited rather than forced private: a management room that
 * is more open than the project it manages would leak, and one that is more
 * closed than an open project would surprise people who could already see
 * everything.
 */
export function managementChannelInput(
  work: CreateChannelInput,
): CreateChannelInput {
  return {
    name: managementChannelNameFor(work.name),
    channelType: work.channelType,
    visibility: work.visibility,
    description: `Management room for #${work.name.trim()}. The owner and the reporting line only; delivery work happens in the project channel.`,
  };
}

/** Should a management room be made for this channel at all? */
export function wantsManagementChannel(input: CreateChannelInput): boolean {
  // Only for a channel that is actually a project: a folder bound to it is
  // what makes a management room worth having. A casual conversation channel
  // paired with an empty management room is clutter.
  if (input.localProject == null) return false;
  // Never pair a management room with another one, which is what an
  // unguarded rule would do when the owner names a channel that way himself.
  return !isManagementChannel(input.name.trim());
}
