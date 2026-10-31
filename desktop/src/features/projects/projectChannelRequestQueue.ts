import type { ProjectChannelRequest } from "@/features/projects/projectChannelRequest";

export const MAX_PENDING_PROJECT_CHANNEL_REQUESTS = 100;
const MAX_SEEN_PROJECT_CHANNEL_REQUEST_IDS =
  MAX_PENDING_PROJECT_CHANNEL_REQUESTS * 2 + 1;

/**
 * Any owner-reviewed request an agent sends: the queue only needs its id.
 * Folder projects (`folderProjectRequest.ts`) reuse this queue as-is.
 */
type IdentifiedRequest = { requestId: string };

export type AcceptedProjectChannelRequest<
  R extends IdentifiedRequest = ProjectChannelRequest,
> = {
  agentPubkey: string;
  request: R;
};

export type ProjectChannelRequestQueue<
  R extends IdentifiedRequest = ProjectChannelRequest,
> = {
  activeRequestId: string | null;
  pending: AcceptedProjectChannelRequest<R>[];
  seenRequestIds: Set<string>;
};

export type EnqueueProjectChannelRequestResult<
  R extends IdentifiedRequest = ProjectChannelRequest,
> =
  | { status: "show"; candidate: AcceptedProjectChannelRequest<R> }
  | { status: "queued" | "duplicate" | "overflow" };

export function createProjectChannelRequestQueue<
  R extends IdentifiedRequest = ProjectChannelRequest,
>(): ProjectChannelRequestQueue<R> {
  return {
    activeRequestId: null,
    pending: [],
    seenRequestIds: new Set(),
  };
}

export function enqueueProjectChannelRequest<R extends IdentifiedRequest>(
  queue: ProjectChannelRequestQueue<R>,
  candidate: AcceptedProjectChannelRequest<R>,
): EnqueueProjectChannelRequestResult<R> {
  const requestId = candidate.request.requestId;
  if (queue.seenRequestIds.has(requestId)) return { status: "duplicate" };

  if (
    queue.activeRequestId !== null &&
    queue.pending.length >= MAX_PENDING_PROJECT_CHANNEL_REQUESTS
  ) {
    // Keep the requests already visible to the owner and drop the newest one.
    // Do not mark it seen: a later retry may be accepted after space opens.
    return { status: "overflow" };
  }

  queue.seenRequestIds.add(requestId);
  pruneSeenRequestIds(queue);

  if (queue.activeRequestId === null) {
    queue.activeRequestId = requestId;
    return { status: "show", candidate };
  }

  queue.pending.push(candidate);
  return { status: "queued" };
}

export function advanceProjectChannelRequestQueue<R extends IdentifiedRequest>(
  queue: ProjectChannelRequestQueue<R>,
): AcceptedProjectChannelRequest<R> | null {
  const next = queue.pending.shift() ?? null;
  queue.activeRequestId = next?.request.requestId ?? null;
  pruneSeenRequestIds(queue);
  return next;
}

function pruneSeenRequestIds<R extends IdentifiedRequest>(
  queue: ProjectChannelRequestQueue<R>,
) {
  if (queue.seenRequestIds.size <= MAX_SEEN_PROJECT_CHANNEL_REQUEST_IDS) return;

  const pendingIds = new Set(
    queue.pending.map((candidate) => candidate.request.requestId),
  );
  for (const requestId of queue.seenRequestIds) {
    if (requestId !== queue.activeRequestId && !pendingIds.has(requestId)) {
      queue.seenRequestIds.delete(requestId);
      if (queue.seenRequestIds.size <= MAX_SEEN_PROJECT_CHANNEL_REQUEST_IDS) {
        return;
      }
    }
  }
}
