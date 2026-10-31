import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import {
  decideProjectApprovalRequest,
  listProjectApprovalRequests,
  type ApprovalRequest,
  type ProjectTeamScope,
} from "@/shared/api/channelProjectTeam";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

const categoryLabel: Record<ApprovalRequest["category"], string> = {
  execute: "Command",
  edit: "File edit",
  read: "File read",
  fetch: "Network fetch",
  commit: "Commit or merge",
  publish: "Publish, deploy or transfer",
  channel: "Channel management",
  other: "Other",
};

export function ChannelProjectApprovals({
  scope,
  teamGeneration,
  agentNames,
}: {
  scope: ProjectTeamScope;
  teamGeneration: number | null;
  agentNames: Record<string, string>;
}) {
  const client = useQueryClient();
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [addChecks, setAddChecks] = useState<Record<string, boolean>>({});
  const queryKey = [
    "project-approvals",
    scope.expectedRelay,
    scope.expectedOwner,
    scope.channelId,
  ];
  const query = useQuery({
    queryKey,
    queryFn: () => listProjectApprovalRequests(scope),
    retry: false,
    staleTime: 0,
    refetchInterval: 10_000,
  });
  const decide = useMutation({
    mutationFn: (input: { request: ApprovalRequest; approve: boolean }) => {
      if (query.isError || query.isFetching)
        throw new Error("Refresh the approval queue before deciding.");
      const addToApprovedChecks =
        input.approve &&
        input.request.category === "execute" &&
        Boolean(addChecks[input.request.request_id]);
      if (addToApprovedChecks && teamGeneration === null)
        throw new Error("Reload the team before adding an approved check.");
      return decideProjectApprovalRequest(scope, {
        requestId: input.request.request_id,
        approve: input.approve,
        note: notes[input.request.request_id]?.trim() || undefined,
        addToApprovedChecks,
        expectedTeamGeneration: teamGeneration,
      });
    },
    onSuccess: async () => {
      await Promise.all([
        client.invalidateQueries({ queryKey }),
        client.invalidateQueries({
          queryKey: [
            "channel-project-team",
            scope.expectedRelay,
            scope.expectedOwner,
            scope.channelId,
          ],
        }),
      ]);
    },
  });
  const pending = (query.data ?? []).filter(
    (request) => request.state === "pending",
  );
  const decided = (query.data ?? []).filter(
    (request) => request.state !== "pending",
  );
  return (
    <section aria-label="Project approvals" className="space-y-3 text-sm">
      <h3 className="font-semibold">Actions awaiting your decision</h3>
      <p>
        The harness gate held these actions for you. An agent still waiting —
        for up to five minutes after asking — goes ahead as soon as you approve.
        Once it has stopped waiting, approving a command can still add it to the
        project's approved checks, so the next attempt runs without asking.
        Publishing, deploying and other destructive actions stay per-action
        decisions.
      </p>
      {query.isPending && <p>Loading approval queue…</p>}
      {query.error && <p role="alert">{String(query.error)}</p>}
      {decide.error && <p role="alert">{String(decide.error)}</p>}
      {query.data && pending.length === 0 && <p>No pending requests.</p>}
      <ul aria-label="Pending approval requests" className="space-y-2">
        {pending.map((request) => (
          <li className="space-y-2 rounded border p-3" key={request.request_id}>
            <p className="font-medium">
              {categoryLabel[request.category]} requested by{" "}
              {agentNames[request.agent_pubkey] ?? request.agent_pubkey}
            </p>
            <p className="break-all">
              <bdi dir="ltr">{request.summary}</bdi>
            </p>
            <p className="text-xs text-muted-foreground">
              {request.reason} ·{" "}
              {new Date(request.requested_at * 1000).toLocaleString()}
            </p>
            {request.category === "execute" && (
              <label className="flex items-start gap-2">
                <input
                  checked={Boolean(addChecks[request.request_id])}
                  onChange={(event) =>
                    setAddChecks({
                      ...addChecks,
                      [request.request_id]: event.target.checked,
                    })
                  }
                  type="checkbox"
                />
                <span>
                  Add this exact command line to the approved checks (any member
                  with the run_checks grant may run it from now on).
                </span>
              </label>
            )}
            <label
              className="block space-y-1"
              htmlFor={`approval-note-${request.request_id}`}
            >
              <span>Decision note (optional)</span>
              <Input
                id={`approval-note-${request.request_id}`}
                onChange={(event) =>
                  setNotes({
                    ...notes,
                    [request.request_id]: event.target.value,
                  })
                }
                value={notes[request.request_id] ?? ""}
              />
            </label>
            <div className="flex flex-wrap gap-2">
              <Button
                disabled={decide.isPending || query.isFetching}
                onClick={() => decide.mutate({ request, approve: true })}
                type="button"
              >
                Approve
              </Button>
              <Button
                disabled={decide.isPending || query.isFetching}
                onClick={() => decide.mutate({ request, approve: false })}
                type="button"
                variant="outline"
              >
                Deny
              </Button>
            </div>
          </li>
        ))}
      </ul>
      {decided.length > 0 && (
        <details>
          <summary>Decided requests ({decided.length})</summary>
          <ul className="space-y-1 text-xs">
            {decided.map((request) => (
              <li key={request.request_id}>
                {request.state}: {categoryLabel[request.category]}{" "}
                <bdi dir="ltr">{request.summary}</bdi>
                {request.decision_note ? ` — ${request.decision_note}` : ""}
              </li>
            ))}
          </ul>
        </details>
      )}
      <Button
        disabled={query.isFetching}
        onClick={() => {
          decide.reset();
          void query.refetch();
        }}
        type="button"
        variant="outline"
      >
        Refresh approvals
      </Button>
    </section>
  );
}
