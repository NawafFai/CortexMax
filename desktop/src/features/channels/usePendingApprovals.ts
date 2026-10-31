import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  listPendingApprovalRequests,
  type ApprovalRequest,
} from "@/shared/api/channelProjectTeam";

/**
 * Actions an agent is holding open for the owner's decision.
 *
 * A gated action no longer fails the moment it is refused: the agent's tool
 * call waits a few minutes for the owner to decide. That only helps if the
 * owner hears about it in time, so the queue is read every few seconds — a
 * query against the local database, not the network.
 */
const POLL_MS = 3_000;

export const PENDING_APPROVALS_QUERY_KEY = "pending-project-approvals";

const NO_REQUESTS: ApprovalRequest[] = [];

export function usePendingApprovals() {
  const { activeCommunity } = useCommunities();
  const owner = useIdentityQuery().data?.pubkey;
  const relay = activeCommunity?.relayUrl;
  const query = useQuery({
    queryKey: [PENDING_APPROVALS_QUERY_KEY, relay, owner],
    queryFn: () =>
      listPendingApprovalRequests({
        expectedRelay: relay as string,
        expectedOwner: owner as string,
      }),
    enabled: Boolean(relay && owner),
    refetchInterval: POLL_MS,
    retry: false,
  });
  return {
    owner,
    relay,
    requests: query.data ?? NO_REQUESTS,
    /** Whether the queue has been read at least once. */
    loaded: query.isSuccess,
  };
}

/** How many requests are waiting in each channel, by channel id. */
export function usePendingApprovalCounts(): ReadonlyMap<string, number> {
  const { requests } = usePendingApprovals();
  return React.useMemo(() => {
    const counts = new Map<string, number>();
    for (const request of requests) {
      counts.set(request.channel_id, (counts.get(request.channel_id) ?? 0) + 1);
    }
    return counts;
  }, [requests]);
}
