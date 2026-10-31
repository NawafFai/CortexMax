import { workChannelNameFor } from "@/features/channels/lib/projectScaffold";

/**
 * Every project gets its own folder in the sidebar, without the owner making
 * one.
 *
 * A project is two rooms — the work channel and its `-management` room — and
 * a team running several projects fills one flat list with pairs whose halves
 * drift apart as they sort. Custom sections already fold and hold channels;
 * what was missing is that nobody makes them. So each project is put into a
 * folder named after it, the first time it is seen.
 *
 * Only once. The owner's own arrangement always wins: a project whose rooms
 * the owner has already placed is left where it is, and a project that has
 * been foldered is remembered as settled, so moving its rooms out — or
 * deleting the folder — is final rather than undone on the next render.
 */

/** The emoji a project folder is created with. */
export const PROJECT_SECTION_ICON = "📁";

export type ProjectPair = {
  /** The work channel's name, which the folder is named after. */
  name: string;
  workId: string;
  managementId: string;
};

type ChannelLike = {
  id: string;
  name: string;
  channelType: string;
  archivedAt?: string | null;
};

type SectionLike = { id: string; name: string };

/** Projects in a channel list: live work channels paired with their room. */
export function findProjectPairs(
  channels: readonly ChannelLike[],
): ProjectPair[] {
  const live = new Map<string, ChannelLike>();
  for (const channel of channels) {
    if (channel.channelType === "dm" || channel.archivedAt) continue;
    live.set(channel.name, channel);
  }
  const pairs: ProjectPair[] = [];
  for (const channel of live.values()) {
    const workName = workChannelNameFor(channel.name);
    if (workName == null) continue;
    const work = live.get(workName);
    if (work == null || work.id === channel.id) continue;
    pairs.push({ name: workName, workId: work.id, managementId: channel.id });
  }
  return pairs.sort((left, right) => left.name.localeCompare(right.name));
}

export type ProjectSectionPlan = {
  /** Folders to create, each with the rooms that go in it. */
  create: { name: string; channelIds: string[] }[];
  /** Rooms to put into a folder that already exists under the project's name. */
  assign: { channelId: string; sectionId: string }[];
  /** Projects now arranged — by this or by the owner — never to be touched again. */
  settled: string[];
};

/**
 * What to do for the projects not yet settled.
 *
 * Pure, and idempotent while its instructions are pending: a pass that runs
 * again before the created folder shows up in `sections` finds the folder by
 * name the next time and assigns into it instead of making a second one.
 */
export function planProjectSections(
  pairs: readonly ProjectPair[],
  sections: readonly SectionLike[],
  assignments: Readonly<Record<string, string>>,
  settled: ReadonlySet<string>,
): ProjectSectionPlan {
  const plan: ProjectSectionPlan = { create: [], assign: [], settled: [] };
  for (const pair of pairs) {
    if (settled.has(pair.workId)) continue;
    const folder = sections.find((section) => section.name === pair.name);
    const rooms = [pair.workId, pair.managementId];
    const placed = rooms.map((id) => assignments[id]);

    if (placed.every((sectionId) => sectionId == null)) {
      if (folder) {
        for (const channelId of rooms) {
          plan.assign.push({ channelId, sectionId: folder.id });
        }
      } else {
        plan.create.push({ name: pair.name, channelIds: rooms });
      }
      continue;
    }

    // A room anywhere but the project's own folder was put there by the
    // owner: the whole project is theirs to arrange from here on.
    if (
      placed.some((sectionId) => sectionId != null && sectionId !== folder?.id)
    ) {
      plan.settled.push(pair.workId);
      continue;
    }
    // Otherwise part of the pair is already in its folder: finish it, and
    // settle once both rooms are in.
    const missing = rooms.filter((_, index) => placed[index] == null);
    if (folder == null || missing.length === 0) {
      plan.settled.push(pair.workId);
      continue;
    }
    for (const channelId of missing) {
      plan.assign.push({ channelId, sectionId: folder.id });
    }
  }
  return plan;
}
