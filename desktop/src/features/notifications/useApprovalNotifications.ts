import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { APPROVAL_ASKS } from "@/features/channels/lib/approvalCopy";
import { usePendingApprovals } from "@/features/channels/usePendingApprovals";
import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { sendDesktopNotification } from "./lib/desktop";
import type { NotificationSettings } from "./hooks";

/** Longest command shown in a notification body before it is cut. */
const SUMMARY_PREVIEW_CHARS = 120;

/**
 * Tell the owner, wherever they are, that an agent is waiting on them.
 *
 * The banner on the composer only helps an owner looking at that channel.
 * A held action waits minutes, not hours, so an owner who is elsewhere needs
 * the nudge a mention would get. Each request is announced once.
 */
export function useApprovalNotifications(
  settings: NotificationSettings,
  channels: ReadonlyArray<Pick<Channel, "id" | "name">>,
) {
  const { requests, loaded } = usePendingApprovals();
  const agents = useManagedAgentsQuery().data;
  const announced = React.useRef<Set<string> | null>(null);

  React.useEffect(() => {
    if (!loaded) return;
    // The first read only records what was already waiting, so opening the
    // app onto an old queue does not fire a burst of stale alerts.
    if (announced.current === null) {
      announced.current = new Set(
        requests.map((request) => request.request_id),
      );
      return;
    }
    const seen = announced.current;
    for (const request of requests) {
      if (seen.has(request.request_id)) continue;
      seen.add(request.request_id);
      if (!settings.desktopEnabled) continue;
      const agentName =
        agents?.find(
          (agent) =>
            normalizePubkey(agent.pubkey) ===
            normalizePubkey(request.agent_pubkey),
        )?.name ?? "An agent";
      const channelName =
        channels.find((channel) => channel.id === request.channel_id)?.name ??
        null;
      const summary =
        request.summary.length > SUMMARY_PREVIEW_CHARS
          ? `${request.summary.slice(0, SUMMARY_PREVIEW_CHARS)}…`
          : request.summary;
      void sendDesktopNotification({
        title: `${agentName} needs your approval`,
        body: `${channelName ? `#${channelName} — ` : ""}wants to ${APPROVAL_ASKS[request.category]}: ${summary}`,
        target: {
          channelId: request.channel_id,
          channelName,
          eventId: null,
          kind: null,
        },
      });
    }
  }, [agents, channels, loaded, requests, settings.desktopEnabled]);
}
