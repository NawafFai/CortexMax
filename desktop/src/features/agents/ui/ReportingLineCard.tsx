import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { updateManagedAgent } from "@/shared/api/tauri";
import type { ManagedAgent } from "@/shared/api/types";

/**
 * Who reports to whom, on one screen.
 *
 * The chart used to exist only as sentences inside each agent's prompt, which
 * meant the code could not act on it: copying a manager on a cross-department
 * request was a rule every agent had to remember, and one that forgot sent a
 * second `@mention` instead — waking the manager it meant merely to inform.
 *
 * Set here, the line becomes data. `buzz messages send` reads it and copies
 * the sender's manager whenever a message leaves its department, whatever
 * model is driving and whether or not it remembered the policy.
 *
 * It is one screen rather than a field inside each agent's dialog because a
 * chart is a set of relationships, and drawing thirty of them through thirty
 * dialogs is how a chart ends up half-drawn.
 */

/** An agent cannot manage itself, and cannot manage its own manager's chain. */
function wouldCycle(
  agent: string,
  candidate: string,
  managerOf: Map<string, string | null>,
): boolean {
  let cursor: string | null = candidate;
  const seen = new Set<string>();
  while (cursor) {
    if (cursor === agent) return true;
    if (seen.has(cursor)) return false;
    seen.add(cursor);
    cursor = managerOf.get(cursor) ?? null;
  }
  return false;
}

function AgentRow({
  agent,
  agents,
  managerOf,
  onChange,
  pending,
}: {
  agent: ManagedAgent;
  agents: ManagedAgent[];
  managerOf: Map<string, string | null>;
  onChange: (pubkey: string, manager: string | null) => void;
  pending: boolean;
}) {
  const options = agents.filter(
    (candidate) =>
      candidate.pubkey !== agent.pubkey &&
      !wouldCycle(agent.pubkey, candidate.pubkey, managerOf),
  );

  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1.5 px-4 py-2.5">
      <span className="min-w-0 flex-1 truncate text-sm text-foreground">
        {agent.name}
      </span>
      <select
        aria-label={`Who ${agent.name} reports to`}
        className="h-8 min-w-0 max-w-[18rem] flex-1 rounded-md border border-input bg-background px-2 text-sm text-foreground"
        disabled={pending}
        onChange={(event) => onChange(agent.pubkey, event.target.value || null)}
        value={agent.managerPubkey ?? ""}
      >
        <option value="">Reports to you</option>
        {options.map((candidate) => (
          <option key={candidate.pubkey} value={candidate.pubkey}>
            {candidate.name}
          </option>
        ))}
      </select>
    </li>
  );
}

export function ReportingLineCard() {
  const agentsQuery = useManagedAgentsQuery();
  const client = useQueryClient();

  const agents = React.useMemo(
    () =>
      [...(agentsQuery.data ?? [])].sort((left, right) =>
        left.name.localeCompare(right.name),
      ),
    [agentsQuery.data],
  );

  const managerOf = React.useMemo(
    () => new Map(agents.map((agent) => [agent.pubkey, agent.managerPubkey])),
    [agents],
  );

  const mutation = useMutation({
    mutationFn: ({
      pubkey,
      managerPubkey,
    }: {
      pubkey: string;
      managerPubkey: string | null;
    }) => updateManagedAgent({ pubkey, managerPubkey }),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: ["managed-agents"] });
      // Said once, not per row: a subscription is read at launch, so the
      // copy policy a new line describes reaches an agent on its next start.
      toast.success("Reporting line saved. Restart the agents to apply.");
    },
    onError: (cause: unknown) =>
      toast.error(cause instanceof Error ? cause.message : String(cause)),
  });

  const placed = agents.filter((agent) => agent.managerPubkey).length;

  return (
    <SettingsOptionGroup
      title="Reporting line"
      description={
        agents.length === 0
          ? "No agents yet."
          : `${placed} of ${agents.length} placed. An agent that reports to you is treated as top of its own chain — a message it sends is copied to nobody.`
      }
    >
      {agentsQuery.isLoading ? (
        <p className="px-4 py-3 text-sm text-muted-foreground">Loading…</p>
      ) : (
        <ul className="divide-y divide-border/60">
          {agents.map((agent) => (
            <AgentRow
              agent={agent}
              agents={agents}
              key={agent.pubkey}
              managerOf={managerOf}
              onChange={(pubkey, managerPubkey) =>
                mutation.mutate({ managerPubkey, pubkey })
              }
              pending={mutation.isPending}
            />
          ))}
        </ul>
      )}
    </SettingsOptionGroup>
  );
}
