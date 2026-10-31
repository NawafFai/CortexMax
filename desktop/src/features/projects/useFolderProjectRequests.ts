import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { classifyAgentManagementOrigin } from "@/features/agents/agentManagementBuffer";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { subscribeFolderProjectRequests } from "@/features/agents/observerRelayStore";
import { channelsQueryKey, useChannelsQuery } from "@/features/channels/hooks";
import { createChannelWithScaffold } from "@/features/channels/lib/createChannelWithScaffold";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  folderProjectCreatedNote,
  type FolderProjectRequest,
} from "@/features/projects/folderProjectRequest";
import {
  advanceProjectChannelRequestQueue,
  createProjectChannelRequestQueue,
  enqueueProjectChannelRequest,
  type AcceptedProjectChannelRequest,
} from "@/features/projects/projectChannelRequestQueue";
import { useIdentityQuery } from "@/shared/api/hooks";
import { addChannelMembers } from "@/shared/api/tauri";
import {
  listManagedAgentRuntimes,
  startManagedAgentRuntime,
  stopManagedAgentRuntime,
} from "@/shared/api/tauriManagedAgents";
import { checkAgentProjectFolder } from "@/shared/api/tauriAgentProjects";
import { sendChannelMessage } from "@/shared/api/tauriMessages";
import type {
  Channel,
  ManagedAgentRuntimeLifecycle,
  ManagedAgentRuntimeStatus,
} from "@/shared/api/types";
import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";
import { normalizePubkey } from "@/shared/lib/pubkey";

type Accepted = AcceptedProjectChannelRequest<FolderProjectRequest>;

/** One agent in one community, as the runtime commands address it. */
export type RuntimePair = { pubkey: string; relayUrl: string };

/** Lifecycles in which an agent's process is up and must stop for a binding. */
const UP: ReadonlySet<ManagedAgentRuntimeLifecycle> = new Set([
  "starting",
  "listening",
  "waking",
  "ready",
]);

/**
 * The agents of this community whose process is up. Binding a folder refuses
 * while any of them run; a failed or stopped one has no process to stop.
 */
export function runningPairs(
  statuses: readonly ManagedAgentRuntimeStatus[],
  relayUrl: string,
): RuntimePair[] {
  const community = normalizeRelayUrl(relayUrl);
  return statuses
    .filter(
      (status) =>
        UP.has(status.lifecycle) &&
        normalizeRelayUrl(status.relayUrl) === community,
    )
    .map((status) => ({ pubkey: status.pubkey, relayUrl: status.relayUrl }));
}

/** What the folder check said about the requested folder. */
export type FolderCheck =
  | { status: "checking" }
  | { status: "ok"; root: string }
  | { status: "refused"; reason: string };

/** Where a confirmed request is in its run, for the button's label. */
export type FolderProjectPhase = "idle" | "pausing" | "creating" | "resuming";

/**
 * Owner review for a project an agent asked to start.
 *
 * Confirming does exactly what the owner's own create dialog does — work
 * channel bound to the folder, management room, handover note — then makes
 * the agent an admin of both rooms and tells it where they are, in the
 * channel it asked from.
 *
 * Binding a folder requires the community's agents to be stopped: each one
 * takes a snapshot of the project bindings when it starts, and a binding that
 * changed under a running agent would never reach it. So the agents running
 * now are stopped one by one and started again afterwards — whatever happened
 * in between, so a failed creation never leaves them stopped — and no other
 * agent is touched: one the owner paused stays paused.
 */
export function useFolderProjectRequests() {
  const queryClient = useQueryClient();
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const { activeCommunity } = useCommunities();
  const [current, setCurrent] = React.useState<Accepted | null>(null);
  const [folderCheck, setFolderCheck] = React.useState<FolderCheck>({
    status: "checking",
  });
  const [phase, setPhase] = React.useState<FolderProjectPhase>("idle");
  const [error, setError] = React.useState<string | null>(null);
  const queue = React.useRef(
    createProjectChannelRequestQueue<FolderProjectRequest>(),
  );
  const buffered = React.useRef<Accepted[]>([]);
  const managedAgentsRef = React.useRef(managedAgentsQuery.data);
  const channelsRef = React.useRef(channelsQuery.data);

  const accept = React.useEffectEvent((candidate: Accepted) => {
    if (
      classifyAgentManagementOrigin(
        managedAgentsRef.current,
        channelsRef.current,
        candidate.agentPubkey,
        candidate.request.request.channelId,
      ) !== "accept"
    ) {
      return;
    }
    const result = enqueueProjectChannelRequest(queue.current, candidate);
    if (result.status === "show") show(result.candidate);
  });

  React.useEffect(() => {
    managedAgentsRef.current = managedAgentsQuery.data;
    channelsRef.current = channelsQuery.data;
    if (!managedAgentsQuery.data || !channelsQuery.data) return;
    for (const candidate of buffered.current.splice(0)) accept(candidate);
  }, [channelsQuery.data, managedAgentsQuery.data]);

  React.useEffect(
    () =>
      subscribeFolderProjectRequests((agentPubkey, request) => {
        const candidate = { agentPubkey, request };
        const origin = classifyAgentManagementOrigin(
          managedAgentsRef.current,
          channelsRef.current,
          agentPubkey,
          request.request.channelId,
        );
        if (origin === "buffer") {
          buffered.current.push(candidate);
          if (buffered.current.length > 100) buffered.current.shift();
          return;
        }
        accept(candidate);
      }),
    [],
  );

  // Check the folder the moment a request is shown, so the owner reads the
  // verdict alongside the path rather than after pressing Create.
  const folder = current?.request.request.folder ?? null;
  React.useEffect(() => {
    if (folder == null) return;
    let live = true;
    setFolderCheck({ status: "checking" });
    checkAgentProjectFolder(folder).then(
      (root) => {
        if (live) setFolderCheck({ status: "ok", root });
      },
      (reason: unknown) => {
        if (live) {
          setFolderCheck({
            status: "refused",
            reason: reason instanceof Error ? reason.message : String(reason),
          });
        }
      },
    );
    return () => {
      live = false;
    };
  }, [folder]);

  function show(candidate: Accepted | null) {
    setError(null);
    setPhase("idle");
    setCurrent(candidate);
  }

  const agentName =
    current == null
      ? null
      : (managedAgentsQuery.data?.find(
          (agent) =>
            normalizePubkey(agent.pubkey) ===
            normalizePubkey(current.agentPubkey),
        )?.name ?? "Your agent");

  function dismiss() {
    if (phase !== "idle") return;
    show(advanceProjectChannelRequestQueue(queue.current));
  }

  async function approve() {
    if (!current || phase !== "idle") return;
    if (folderCheck.status !== "ok") {
      setError(
        folderCheck.status === "refused"
          ? folderCheck.reason
          : "The folder is still being checked.",
      );
      return;
    }
    const owner = identityQuery.data?.pubkey;
    const relay = activeCommunity?.relayUrl;
    if (!owner || !relay) {
      setError("Your identity is still loading. Try again in a moment.");
      return;
    }
    const { request } = current.request;
    const agent = current.agentPubkey;
    setError(null);

    // Exactly the agents running now are stopped, and exactly they come back.
    // The Break record is left alone: resuming it would also start agents the
    // owner paused on purpose, some of them a day earlier.
    const stopped: RuntimePair[] = [];
    let created: { channel: Channel; management: Channel | null } | null = null;
    let adminFailures = 0;
    try {
      const running = runningPairs(await listManagedAgentRuntimes(), relay);
      if (running.length > 0) setPhase("pausing");
      for (const pair of running) {
        await stopManagedAgentRuntime(pair.pubkey, pair.relayUrl);
        stopped.push(pair);
      }
      setPhase("creating");
      created = await createChannelWithScaffold(
        {
          name: request.name,
          channelType: "stream",
          visibility: request.visibility,
          description: request.description,
          localProject: {
            requestId: crypto.randomUUID(),
            root: folderCheck.root,
            expectedOwner: owner,
            expectedRelay: relay,
          },
        },
        { relayUrl: relay, ownerPubkey: owner },
      );
      for (const room of [created.channel, created.management]) {
        if (room == null) continue;
        try {
          const added = await addChannelMembers({
            channelId: room.id,
            pubkeys: [agent],
            role: "admin",
            expectedRelayUrl: relay,
            expectedSignerPubkey: owner,
          });
          if (added.errors.length > 0) adminFailures += 1;
        } catch {
          adminFailures += 1;
        }
      }
      void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (stopped.length > 0) {
        setPhase("resuming");
        let notStarted = 0;
        for (const pair of stopped) {
          try {
            await startManagedAgentRuntime(pair.pubkey, pair.relayUrl);
          } catch {
            notStarted += 1;
          }
        }
        if (notStarted > 0) {
          toast.error(
            `${notStarted} agent(s) did not start again. Start them from the Agents page.`,
          );
        }
      }
      setPhase("idle");
    }
    if (created == null) return;

    // Told where it asked, so the agent learns the ids without searching and
    // the owner sees what was made in the same thread.
    try {
      await sendChannelMessage(
        request.channelId,
        folderProjectCreatedNote({
          agentName: agentName ?? "agent",
          workName: created.channel.name,
          workId: created.channel.id,
          managementName: created.management?.name ?? null,
          managementId: created.management?.id ?? null,
          folder: folderCheck.root,
        }),
        null,
        undefined,
        [agent],
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        relay,
        owner,
      );
    } catch {
      toast.warning(
        `Project created, but the note to ${agentName ?? "the agent"} could not be sent.`,
      );
    }
    if (adminFailures > 0) {
      toast.warning(
        `Project created, but ${agentName ?? "the agent"} could not be made an admin of every room. Add it from the channel's members.`,
      );
    }
    toast.success(`Project #${created.channel.name} created.`);
    show(advanceProjectChannelRequestQueue(queue.current));
  }

  return {
    agentName,
    approve,
    dismiss,
    error,
    folderCheck,
    phase,
    request: current?.request ?? null,
  };
}
