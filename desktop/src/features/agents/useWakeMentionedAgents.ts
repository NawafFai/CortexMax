import * as React from "react";

import { useDetachedAgentStart } from "@/features/messages/ui/useDetachedAgentStart";
import { isManagedAgentRunning } from "@/features/messages/ui/useMentionSendFlow.helpers";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { invokeTauri } from "@/shared/api/tauri";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Start an agent that was `@`-mentioned while it was stopped.
 *
 * A mention is supposed to wake an agent — that is what makes it safe to keep
 * most of a team idle. It already worked for messages the owner sent from the
 * app, because the send path checks the mentioned agents first. It did not
 * work for one agent mentioning another, because those go straight from the
 * CLI to the relay and the app never sees them on the way out.
 *
 * Which is exactly the case the reporting line creates: a specialist asking
 * another department for something. The request landed, the agent it named
 * was stopped, and nothing happened at all — no reply, no error, no turn to
 * report on. The person who asked saw the same silence as a broken agent.
 *
 * The home feed cannot answer this: it queries the relay for messages that
 * name the OWNER, so a request from one specialist to another department is
 * simply not in it. The relay is asked directly instead — which of my agents
 * were named recently — and copies are excluded there, so being kept in the
 * loop still costs nothing.
 */

/** How often to ask the relay who was named. */
const POLL_MS = 20_000;

/** How far back each poll looks. Wider than the interval so a slow tick
 * cannot skip a mention, and narrow enough that history wakes nobody. */
const LOOKBACK_SECONDS = 120;

type AgentMention = { agentPubkey: string; earliestAt: number };

export function useWakeMentionedAgents() {
  const agentsQuery = useManagedAgentsQuery();
  const startAgentDetached = useDetachedAgentStart();
  // One attempt per (agent, mention) pair. A start that fails is not retried
  // every twenty seconds, which would hammer a genuinely broken agent.
  const attemptedRef = React.useRef<Set<string>>(new Set());
  const agents = agentsQuery.data;
  const agentsRef = React.useRef(agents);
  agentsRef.current = agents;

  React.useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      const roster = agentsRef.current;
      if (!roster || roster.length === 0) return;
      let mentions: AgentMention[];
      try {
        mentions = await invokeTauri<AgentMention[]>("recent_agent_mentions", {
          since: Math.floor(Date.now() / 1000) - LOOKBACK_SECONDS,
        });
      } catch {
        // A relay that cannot be reached is not a reason to log every tick.
        return;
      }
      if (cancelled) return;
      const byPubkey = new Map(
        roster.map((agent) => [normalizePubkey(agent.pubkey), agent]),
      );
      for (const mention of mentions) {
        const pubkey = normalizePubkey(mention.agentPubkey);
        const key = `${pubkey}:${mention.earliestAt}`;
        if (attemptedRef.current.has(key)) continue;
        const agent = byPubkey.get(pubkey);
        if (!agent) continue;
        attemptedRef.current.add(key);
        if (isManagedAgentRunning(agent)) continue;
        // The mention's own timestamp is the replay floor, so the agent
        // replays back to the message however long the start takes.
        startAgentDetached(agent, mention.earliestAt);
      }
    };
    void tick();
    const timer = setInterval(() => void tick(), POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [startAgentDetached]);
}
