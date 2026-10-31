import * as React from "react";
import { Pause, Play } from "lucide-react";
import { toast } from "sonner";

import {
  breakManagedAgents,
  managedAgentsBreakState,
  resumeManagedAgents,
} from "@/shared/api/tauri";
import type { BreakOutcome, BreakRecord } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import { formatAgo } from "@/features/agents/lib/teamRoster";
import { truncateNpub } from "@/shared/lib/pubkey";

/**
 * Break and Start: stop every running agent at once, then bring back exactly
 * the ones that were running.
 *
 * Stopping a team one agent at a time is useless in the moment it matters — a
 * run is burning a shared provider limit and the owner wants it to stop now.
 * Quitting the app stops the burn too but forgets what was running, so
 * resuming means rebuilding the list by hand. Break records the pairs it
 * stops; Start replays that record.
 *
 * The control reports partial outcomes rather than claiming success: an agent
 * that refuses to stop stays in the failure list and in the record, so a
 * second press retries it instead of leaving it silently running.
 */
export function BreakResumeControl() {
  const [record, setRecord] = React.useState<BreakRecord | null>(null);
  const [busy, setBusy] = React.useState(false);

  const refresh = React.useCallback(async () => {
    try {
      setRecord(await managedAgentsBreakState());
    } catch {
      // An unreadable record must not disable the Break button — stopping a
      // runaway team matters more than knowing what a previous break held.
      setRecord(null);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  const paused = record?.pairs.length ?? 0;

  function report(outcome: BreakOutcome, verb: "stopped" | "started") {
    setRecord(outcome.record);
    if (outcome.failures.length === 0) {
      toast.success(
        outcome.succeeded === 0
          ? `Nothing to ${verb === "stopped" ? "stop" : "start"}.`
          : `${outcome.succeeded} agent${outcome.succeeded === 1 ? "" : "s"} ${verb}.`,
      );
      return;
    }
    toast.warning(
      `${outcome.succeeded} ${verb}, ${outcome.failures.length} refused: ${
        outcome.failures[0]?.error ?? "unknown error"
      }`,
    );
  }

  async function run(
    action: () => Promise<BreakOutcome>,
    verb: "stopped" | "started",
  ) {
    setBusy(true);
    try {
      report(await action(), verb);
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : String(cause));
      void refresh();
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex items-center gap-2">
      <Button
        aria-label="Stop every running agent"
        disabled={busy}
        onClick={() => void run(breakManagedAgents, "stopped")}
        size="sm"
        type="button"
        variant="outline"
      >
        <Pause />
        <span>Break</span>
      </Button>
      <Button
        aria-label="Start the agents the last break stopped"
        disabled={busy || paused === 0}
        onClick={() => void run(resumeManagedAgents, "started")}
        size="sm"
        type="button"
        variant={paused > 0 ? "default" : "outline"}
      >
        <Play />
        <span>Start{paused > 0 ? ` (${paused})` : ""}</span>
      </Button>
      {paused > 0 ? (
        <span
          className="truncate text-2xs text-muted-foreground"
          title={record?.pairs
            .map((pair) => {
              // Say what each paused agent was doing, not only who it is.
              // After a restart this record is the sole surviving account of
              // the run, and "Start (12)" alone does not say what comes back.
              const who = pair.name ?? truncateNpub(pair.pubkey);
              const on = pair.model ? ` on ${pair.model}` : "";
              const when =
                pair.lastTurnAt == null
                  ? " — no turn on record"
                  : ` — last worked ${formatAgo(Date.now() / 1000 - pair.lastTurnAt)}`;
              return `${who}${on}${when}`;
            })
            .join("\n")}
        >
          {paused} paused
          {record?.pausedAt
            ? ` since ${new Date(record.pausedAt).toLocaleTimeString([], {
                hour: "2-digit",
                minute: "2-digit",
              })}`
            : ""}
        </span>
      ) : null}
    </div>
  );
}
