import * as React from "react";

import type { Channel } from "@/shared/api/types";
import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";
import {
  findProjectPairs,
  PROJECT_SECTION_ICON,
  planProjectSections,
} from "./projectSections";
import type { ChannelSection } from "./useChannelSections";

/** Projects already foldered or arranged by the owner, per identity and relay. */
const SETTLED_KEY_PREFIX = "cortex-max.project-sections-settled.v1";
const MAX_SETTLED = 1_000;

function settledKey(pubkey: string, relayUrl: string): string {
  return `${SETTLED_KEY_PREFIX}:${pubkey}:${encodeURIComponent(
    normalizeRelayUrl(relayUrl),
  )}`;
}

function readSettled(key: string): Set<string> {
  try {
    const parsed: unknown = JSON.parse(
      window.localStorage.getItem(key) ?? "[]",
    );
    return new Set(
      Array.isArray(parsed)
        ? parsed.filter((id): id is string => typeof id === "string")
        : [],
    );
  } catch {
    return new Set();
  }
}

function writeSettled(key: string, settled: Set<string>) {
  try {
    window.localStorage.setItem(
      key,
      JSON.stringify([...settled].slice(-MAX_SETTLED)),
    );
  } catch {
    // Unwritable storage costs a repeat of a pass that changes nothing: the
    // folder already holds its rooms, so the next plan settles it again.
  }
}

/**
 * Put each project into its own sidebar folder, once.
 *
 * See `projectSections.ts` for the rule. This applies it: one folder per pass,
 * so two projects found together never race for the same position, and the
 * next pass — triggered by the folder appearing — takes the next project.
 */
export function useAutoProjectSections({
  pubkey,
  relayUrl,
  channels,
  sections,
  assignments,
  createSection,
  assignChannel,
}: {
  pubkey: string | undefined;
  relayUrl: string | undefined;
  channels: readonly Channel[] | undefined;
  sections: readonly ChannelSection[];
  assignments: Readonly<Record<string, string>>;
  createSection: (name: string, icon?: string) => ChannelSection | null;
  assignChannel: (channelId: string, sectionId: string) => void;
}) {
  // Folders made this session, keyed by name, until they reach `sections`: a
  // pass that runs in between must fill the new folder, not make another.
  const created = React.useRef(new Map<string, string>());

  React.useEffect(() => {
    if (!pubkey || !relayUrl || !channels) return;
    const key = settledKey(pubkey, relayUrl);
    const settled = readSettled(key);
    const known: { id: string; name: string }[] = [...sections];
    for (const [name, id] of created.current) {
      if (!known.some((section) => section.id === id)) known.push({ id, name });
    }

    const plan = planProjectSections(
      findProjectPairs(channels),
      known,
      assignments,
      settled,
    );
    const next = plan.create[0];
    if (next) {
      const section = createSection(next.name, PROJECT_SECTION_ICON);
      if (section) {
        created.current.set(next.name, section.id);
        for (const channelId of next.channelIds) {
          assignChannel(channelId, section.id);
        }
      }
    }
    for (const { channelId, sectionId } of plan.assign) {
      assignChannel(channelId, sectionId);
    }
    if (plan.settled.length > 0) {
      for (const id of plan.settled) settled.add(id);
      writeSettled(key, settled);
    }
  }, [
    pubkey,
    relayUrl,
    channels,
    sections,
    assignments,
    createSection,
    assignChannel,
  ]);
}
