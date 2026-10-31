import * as React from "react";
import { toast } from "sonner";

import {
  applyModelFailover,
  modelFailoverPlan,
  type ModelFailoverReport,
} from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";

/**
 * Move the team off one spent model before moving it off its provider.
 *
 * Claude reports a weekly window per premium model beside an account-wide
 * one, and they can disagree sharply — `Week · Opus` at 100% while
 * `Week · all models` sits at 72%. One model is spent; the subscription is
 * not. The cross-provider failover treats a provider as a single wall, so on
 * its own it would migrate the team elsewhere while most of the provider it
 * is on stands unused.
 *
 * The destination is the closest capability still available rather than the
 * cheapest one. Dropping the team to the cheapest tier is what the cost
 * profile is for, and that is the owner's decision to make deliberately.
 *
 * It never acts on its own. The panel says what it would do; pressing the
 * button is what does it, and the backend re-plans against fresh limits at
 * that moment rather than trusting what is on screen — a window may have
 * reset since it was drawn, and rewriting every affected agent on a stale
 * reading is the expensive mistake here.
 */
export function ModelFailoverControl() {
  const [report, setReport] = React.useState<ModelFailoverReport | null>(null);
  const [busy, setBusy] = React.useState(false);

  const load = React.useCallback(async () => {
    try {
      setReport(await modelFailoverPlan());
    } catch {
      // Unreadable limits are the ordinary state before any runtime has been
      // used; no panel is the honest rendering, not an error.
      setReport(null);
    }
  }, []);

  React.useEffect(() => {
    void load();
  }, [load]);

  async function apply() {
    setBusy(true);
    try {
      const done = await applyModelFailover();
      toast.success(
        done.moves.length > 0
          ? `Moved ${done.moves.length} agent${done.moves.length === 1 ? "" : "s"} to ${done.moves[0].toModel}.`
          : done.summary,
      );
      await load();
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }

  if (report == null) return null;
  const movable = report.moves.length > 0;

  return (
    <div className="space-y-2 px-4 py-3.5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <div className="min-w-0">
          <p className="text-sm font-medium">Model failover</p>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {report.summary}
          </p>
        </div>
        {movable ? (
          <Button
            disabled={busy}
            onClick={() => void apply()}
            size="sm"
            type="button"
          >
            Move {report.moves.length}
          </Button>
        ) : null}
      </div>
      {movable ? (
        <p className="text-3xs leading-relaxed text-muted-foreground">
          {report.moves
            .slice(0, 6)
            .map((entry) => entry.name)
            .join(", ")}
          {report.moves.length > 6
            ? ` and ${report.moves.length - 6} more`
            : ""}
          . Agents already running keep their current model until they next
          start.
        </p>
      ) : null}
    </div>
  );
}
