import * as React from "react";
import { toast } from "sonner";

import {
  channelDeliverables,
  setChannelDeliverables,
  type DeliverableReport,
} from "@/shared/api/tauri";
import { getChannelUsage } from "@/shared/api/tauriArchive";
import type { ChannelUsage } from "@/shared/api/tauriArchive";
import {
  buildBucketBoundaries,
  formatUsd,
} from "@/features/usage/lib/usageWindow";
import { formatAgo } from "@/features/agents/lib/teamRoster";
import { Button } from "@/shared/ui/button";
import { ensureChannelHandover } from "@/shared/api/tauri";
import {
  HANDOVER_FILENAME,
  handoverTemplate,
} from "@/features/channels/lib/handoverTemplate";
import { Textarea } from "@/shared/ui/textarea";

/**
 * What this channel is supposed to produce, and what producing it has cost.
 *
 * The two halves are only useful together. A list of finished files does not
 * say whether they were worth the week, and a spend figure does not say what
 * the week bought. Shown side by side, they answer the question an owner
 * actually has when a channel has been running for days.
 *
 * The file check is a filesystem check and nothing more: it says a file exists
 * and how big it is, never that its contents are any good. An empty file is
 * reported as started, not produced — counting it as done is how a progress
 * bar comes to read 100% over nothing.
 */

/** Days of spend attributed to the deliverables. */
const WINDOW_DAYS = 30;

function StateDot({
  target,
}: {
  target: DeliverableReport["targets"][number];
}) {
  const colour = target.error
    ? "bg-red-500"
    : target.present && (target.sizeBytes ?? 0) > 0
      ? "bg-emerald-500"
      : target.present
        ? "bg-amber-500"
        : "bg-muted-foreground/35";
  return (
    <span
      aria-hidden="true"
      className={`h-2 w-2 shrink-0 rounded-full ${colour}`}
    />
  );
}

/** Bytes at the precision a person reads them. */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function ChannelDeliverablesCard({
  channelId,
  channelName = "this channel",
}: {
  channelId: string;
  channelName?: string;
}) {
  const [report, setReport] = React.useState<DeliverableReport | null>(null);
  const [usage, setUsage] = React.useState<ChannelUsage | null>(null);
  const [editing, setEditing] = React.useState(false);
  const [draft, setDraft] = React.useState("");
  const [busy, setBusy] = React.useState(false);

  const load = React.useCallback(async () => {
    const boundaries = buildBucketBoundaries(WINDOW_DAYS);
    const [targets, spend] = await Promise.all([
      channelDeliverables(channelId).catch(() => null),
      getChannelUsage(
        channelId,
        boundaries[0] ?? 0,
        boundaries[boundaries.length - 1] ?? 0,
      ).catch(() => null),
    ]);
    setReport(targets);
    setUsage(spend);
    setDraft((targets?.targets ?? []).map((t) => t.path).join("\n"));
  }, [channelId]);

  React.useEffect(() => {
    void load();
  }, [load]);

  async function save() {
    setBusy(true);
    try {
      await setChannelDeliverables(
        channelId,
        draft
          .split("\n")
          .map((line) => line.trim())
          .filter(Boolean),
      );
      setEditing(false);
      await load();
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }

  const total = report?.total ?? 0;
  const produced = report?.produced ?? 0;
  const percent = total > 0 ? Math.round((produced / total) * 100) : 0;

  /**
   * Put a handover note in the project folder for the team to keep.
   *
   * Agents coordinate well inside one conversation and badly across turns: a
   * different agent's session never saw this one's history, and a restarted
   * agent has forgotten its own. A file in the project is the one thing they
   * all read.
   *
   * It never overwrites. An existing handover is the team's accumulated
   * account of what is in flight and what must not be built upon — replacing
   * that with a blank template would destroy the content that makes it worth
   * having, so an existing file is reported rather than replaced.
   */
  async function addHandover() {
    setBusy(true);
    try {
      const outcome = await ensureChannelHandover(
        channelId,
        handoverTemplate(channelName, new Date().toISOString().slice(0, 10)),
      );
      if (outcome.error) {
        toast.error(outcome.error);
      } else if (outcome.created) {
        toast.success(`Added ${HANDOVER_FILENAME} to the project folder.`);
      } else {
        toast.info(
          `${HANDOVER_FILENAME} is already there — left untouched so the team keeps what it wrote.`,
        );
      }
      await load();
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <div className="min-w-0">
          <p className="text-sm font-medium">Deliverables</p>
          <p className="mt-0.5 text-xs text-muted-foreground">
            The files this channel is meant to produce, and what producing them
            has cost.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {/*
            Kept as a repair, not a routine step: a new project is seeded with
            its handover at creation, so this only matters for a channel made
            before that, or one whose folder was bound afterwards. It is the
            owner's fallback — day to day the note is the GM's and the
            department heads' to keep, which is what their instructions say.
          */}
          <Button
            disabled={busy}
            onClick={() => void addHandover()}
            size="sm"
            type="button"
            variant="ghost"
          >
            Add handover
          </Button>
          <Button
            disabled={busy}
            onClick={() => setEditing((was) => !was)}
            size="sm"
            type="button"
            variant="outline"
          >
            {editing ? "Cancel" : total > 0 ? "Edit" : "Declare targets"}
          </Button>
        </div>
      </div>

      {editing ? (
        <div className="space-y-2">
          <Textarea
            className="min-h-24 font-mono text-xs"
            onChange={(event) => setDraft(event.target.value)}
            placeholder={"out/report.docx\nout/model.xlsx"}
            value={draft}
          />
          <p className="text-3xs leading-relaxed text-muted-foreground">
            One path per line, relative to this channel&apos;s folder. A path
            that tries to leave that folder is refused.
          </p>
          <Button
            disabled={busy}
            onClick={() => void save()}
            size="sm"
            type="button"
          >
            Save
          </Button>
        </div>
      ) : total === 0 ? (
        <p className="text-xs text-muted-foreground">
          No targets declared, so there is nothing to measure progress against.
        </p>
      ) : (
        <>
          <div className="flex items-baseline justify-between gap-3 text-xs">
            <span className="font-medium">
              {produced} of {total} produced
              {report && report.empty > 0
                ? ` · ${report.empty} still empty`
                : ""}
            </span>
            {/*
              The cost sits with the outcome deliberately: a list of finished
              files does not say whether they were worth the week, and a spend
              figure does not say what the week bought.
            */}
            <span className="shrink-0 tabular-nums text-muted-foreground">
              {usage == null || usage.turns === 0
                ? "no recorded spend"
                : `${formatUsd(usage.costUsd)} over ${usage.turns} turns`}
            </span>
          </div>

          <div
            aria-label="Deliverables produced"
            aria-valuemax={100}
            aria-valuemin={0}
            aria-valuenow={percent}
            className="h-2 w-full overflow-hidden rounded-full bg-muted"
            role="progressbar"
          >
            <div
              className="h-full rounded-full bg-emerald-500 transition-[width] duration-500"
              style={{ width: `${percent}%` }}
            />
          </div>

          {report?.noWorkspace ? (
            <p className="text-3xs text-muted-foreground">
              This channel has no folder bound yet, so its targets cannot be
              checked.
            </p>
          ) : null}

          <ul className="divide-y divide-border/55 rounded border">
            {(report?.targets ?? []).map((target) => (
              <li
                className="flex items-center gap-2.5 px-2.5 py-2 text-xs"
                key={target.path}
              >
                <StateDot target={target} />
                <span className="min-w-0 flex-1 truncate font-mono">
                  {target.path}
                </span>
                <span className="shrink-0 tabular-nums text-muted-foreground">
                  {target.error
                    ? target.error
                    : target.present
                      ? `${formatBytes(target.sizeBytes ?? 0)}${
                          target.modifiedAt
                            ? ` · ${formatAgo(Date.now() / 1000 - target.modifiedAt)}`
                            : ""
                        }`
                      : "not produced yet"}
                </span>
              </li>
            ))}
          </ul>

          {usage != null && usage.turnsWithoutCost > 0 ? (
            <p className="text-3xs leading-relaxed text-muted-foreground">
              {usage.turnsWithoutCost} of these turns reported no price, so the
              figure is a floor rather than the whole cost.
            </p>
          ) : null}
        </>
      )}
    </div>
  );
}
