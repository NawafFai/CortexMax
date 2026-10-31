import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ShieldAlert } from "lucide-react";
import { toast } from "sonner";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  APPROVAL_ASKS,
  APPROVAL_HOLD_SECONDS,
} from "@/features/channels/lib/approvalCopy";
import {
  PENDING_APPROVALS_QUERY_KEY,
  usePendingApprovals,
} from "@/features/channels/usePendingApprovals";
import {
  decideProjectApprovalRequest,
  getChannelProjectTeam,
  type ApprovalRequest,
} from "@/shared/api/channelProjectTeam";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useNow } from "@/shared/lib/useNow";
import { Button } from "@/shared/ui/button";

/**
 * An agent in this channel is waiting on you.
 *
 * Stacked at `z-[5]`: above the dock's glass backdrop (`z-0`, which spans the
 * whole dock and blurs whatever lies beneath it — at `z-0` this banner was
 * unreadable) and below the composer (`z-10`), which still covers the
 * banner's bottom edge so the two read as one tab.
 *
 * The approval queue used to live in the Project dialog's Approvals tab, with
 * no count and no notification, so a refused action sat there unseen while
 * the agent that needed it went quiet. Pending requests now sit on the
 * composer, where the owner is already looking, with the two buttons that
 * decide them. An agent still holding its tool call goes ahead the moment
 * Approve lands.
 */
export function ChannelApprovalBanner({
  channelId,
}: {
  channelId: string | null;
}) {
  const { owner, relay, requests } = usePendingApprovals();
  const agents = useManagedAgentsQuery().data;
  const client = useQueryClient();
  const now = useNow(15_000);
  // The request "always allow" was ticked for. Keyed by id, so the box
  // starts unticked for every new command: "always" is decided per command.
  const [alwaysAllowId, setAlwaysAllowId] = React.useState<string | null>(null);

  // Oldest first: the request that has waited longest is the one decided next.
  const pending = React.useMemo(
    () =>
      requests
        .filter((request) => request.channel_id === channelId)
        .sort((left, right) => left.requested_at - right.requested_at),
    [channelId, requests],
  );
  const current = pending[0];
  const alwaysAllow =
    current !== undefined && alwaysAllowId === current.request_id;

  const decide = useMutation({
    mutationFn: async ({
      request,
      approve,
    }: {
      request: ApprovalRequest;
      approve: boolean;
    }) => {
      const scope = {
        channelId: request.channel_id,
        expectedRelay: relay as string,
        expectedOwner: owner as string,
      };
      const addToApprovedChecks =
        approve && alwaysAllow && request.category === "execute";
      // "Always" adds the line to the team's approved checks, a write fenced
      // by the team generation it was decided against.
      const expectedTeamGeneration = addToApprovedChecks
        ? (await getChannelProjectTeam(scope)).generation
        : null;
      return decideProjectApprovalRequest(scope, {
        requestId: request.request_id,
        approve,
        addToApprovedChecks,
        expectedTeamGeneration,
      });
    },
    onSettled: () =>
      client.invalidateQueries({ queryKey: [PENDING_APPROVALS_QUERY_KEY] }),
    onError: (cause: unknown) =>
      toast.error(cause instanceof Error ? cause.message : String(cause)),
  });

  if (!current || !relay || !owner) return null;

  const agentName =
    agents?.find(
      (agent) =>
        normalizePubkey(agent.pubkey) === normalizePubkey(current.agent_pubkey),
    )?.name ?? "An agent";
  const stillHeld = now / 1000 - current.requested_at < APPROVAL_HOLD_SECONDS;
  const busy = decide.isPending;

  return (
    <section
      aria-label="Waiting for your approval"
      className="relative z-[5] mx-5 -mb-3 space-y-2 rounded-t-2xl border border-b-0 border-amber-500/40 bg-amber-500/15 px-4 pb-5 pt-2.5 text-sm leading-5 text-foreground backdrop-blur-sm"
      data-testid="channel-approval-banner"
    >
      <div className="flex items-start gap-2">
        <ShieldAlert
          aria-hidden
          className="mt-0.5 h-4 w-4 shrink-0 text-amber-600"
        />
        <div className="min-w-0 flex-1 space-y-1">
          <p>
            <span className="font-semibold">{agentName}</span>
            {stillHeld
              ? ` is waiting for your approval to ${APPROVAL_ASKS[current.category]}.`
              : ` asked to ${APPROVAL_ASKS[current.category]}, and has stopped waiting.`}
            {pending.length > 1 ? (
              <span className="text-muted-foreground">
                {" "}
                {pending.length - 1} more after this.
              </span>
            ) : null}
          </p>
          <code
            className="block max-h-24 overflow-auto whitespace-pre-wrap break-all rounded bg-background/60 px-2 py-1 text-xs"
            dir="ltr"
          >
            {current.summary}
          </code>
          <p className="text-xs text-muted-foreground">{current.reason}</p>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2 pl-6">
        <Button
          disabled={busy}
          onClick={() => decide.mutate({ request: current, approve: true })}
          size="sm"
          type="button"
        >
          Approve
        </Button>
        <Button
          disabled={busy}
          onClick={() => decide.mutate({ request: current, approve: false })}
          size="sm"
          type="button"
          variant="outline"
        >
          Reject
        </Button>
        {current.category === "execute" ? (
          <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <input
              checked={alwaysAllow}
              disabled={busy}
              onChange={(event) =>
                setAlwaysAllowId(
                  event.target.checked ? current.request_id : null,
                )
              }
              type="checkbox"
            />
            Always allow this exact command
          </label>
        ) : null}
      </div>
    </section>
  );
}
