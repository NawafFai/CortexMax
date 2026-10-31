import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import {
  acknowledgeChannelProjectPreviousWriterRelease,
  getChannelProjectChanges,
  importChannelProjectHandoffSummary,
  inspectChannelProjectHandoff,
  type HandoffInspection,
  type ProjectChangeReview,
  type ProjectTeamScope,
} from "@/shared/api/channelProjectTeam";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

function invalidateTeam(
  client: ReturnType<typeof useQueryClient>,
  scope: ProjectTeamScope,
) {
  return client.invalidateQueries({
    queryKey: [
      "channel-project-team",
      scope.expectedRelay,
      scope.expectedOwner,
      scope.channelId,
    ],
  });
}

export function ChannelProjectReview({
  scope,
  bindingGeneration,
}: {
  scope: ProjectTeamScope;
  bindingGeneration: number;
}) {
  const client = useQueryClient();
  const [label, setLabel] = useState("");
  const [summary, setSummary] = useState("");
  const [released, setReleased] = useState(false);
  const handoff = useMutation({
    mutationFn: async () => {
      const result = await inspectChannelProjectHandoff(
        scope,
        bindingGeneration,
      );
      if (
        result.channel_id !== scope.channelId ||
        result.binding_generation !== bindingGeneration
      )
        throw new Error("Project binding changed. Inspect again.");
      return result;
    },
    onSuccess: () => invalidateTeam(client, scope),
  });
  const importSummary = useMutation({
    mutationFn: () => {
      if (!label.trim() || !summary.trim())
        throw new Error("Give the imported summary a label and its text.");
      return importChannelProjectHandoffSummary(
        scope,
        bindingGeneration,
        label.trim(),
        summary.trim(),
      );
    },
    onSuccess: async () => {
      setLabel("");
      setSummary("");
      handoff.reset();
      await invalidateTeam(client, scope);
    },
  });
  const acknowledge = useMutation({
    mutationFn: () => {
      if (!released)
        throw new Error(
          "Confirm that you verified the previous writer has stopped.",
        );
      return acknowledgeChannelProjectPreviousWriterRelease(scope);
    },
    onSuccess: async () => {
      setReleased(false);
      handoff.reset();
      await invalidateTeam(client, scope);
    },
  });
  const changes = useMutation({
    mutationFn: async () => {
      const result = await getChannelProjectChanges(scope, bindingGeneration);
      if (
        result.channel_id !== scope.channelId ||
        result.binding_generation !== bindingGeneration
      )
        throw new Error("Project binding changed. Review again.");
      return result;
    },
  });
  const inspection: HandoffInspection | undefined = handoff.data;
  const review: ProjectChangeReview | undefined = changes.data;
  const error =
    handoff.error ?? importSummary.error ?? acknowledge.error ?? changes.error;
  return (
    <section aria-label="Project review" className="space-y-4 text-sm">
      {error && <p role="alert">{String(error)}</p>}
      <section aria-label="Handoff" className="space-y-2 rounded border p-3">
        <h3 className="font-semibold">Handoff from a previous session</h3>
        <p>
          Reads a handoff file inside the project (never another provider's
          conversation history), shows the live Git state next to the
          checkpoint, and lists processes still working inside the folder.
          Uncommitted work is preserved as it is; nothing is reset or stashed.
        </p>
        <Button
          disabled={handoff.isPending}
          onClick={() => handoff.mutate()}
          type="button"
          variant="outline"
        >
          {handoff.isPending ? "Inspecting handoff…" : "Inspect handoff"}
        </Button>
        {inspection && !handoff.isError && (
          <div className="space-y-2" data-testid="handoff-result">
            <p role="status">
              {inspection.record
                ? `${inspection.record.source === "file" ? "Handoff file" : "Imported summary"}: ${inspection.record.label} — captured ${new Date(inspection.record.captured_at * 1000).toLocaleString()} at revision ${inspection.record.git_revision?.slice(0, 12) ?? "unknown"}${inspection.record.dirty_at_capture ? " (dirty)" : ""}.`
                : `No handoff file found (probed ${inspection.probed.join(", ")}).`}
            </p>
            <p>
              Live now: branch{" "}
              <bdi dir="ltr">{inspection.live.branch ?? "—"}</bdi>, revision{" "}
              <bdi dir="ltr">
                {inspection.live.revision?.slice(0, 12) ?? "—"}
              </bdi>
              ,{" "}
              {inspection.live.dirty === null
                ? "dirty state unknown"
                : inspection.live.dirty
                  ? `${inspection.live.changed_paths.length} changed path(s) preserved`
                  : "clean"}
              .
              {inspection.record?.git_revision &&
              inspection.live.revision &&
              inspection.record.git_revision !== inspection.live.revision
                ? " The checkpoint predates the current revision."
                : ""}
            </p>
            {inspection.unfinished_items.length > 0 && (
              <div>
                <p className="font-medium">Unfinished items in the handoff</p>
                <ul className="list-disc ps-5">
                  {inspection.unfinished_items.map((item) => (
                    <li dir="auto" key={item}>
                      {item}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {inspection.referenced_paths.length > 0 && (
              <p>
                Evidence paths that exist:{" "}
                {inspection.referenced_paths.map((path) => (
                  <code className="me-2" key={path}>
                    <bdi dir="ltr">{path}</bdi>
                  </code>
                ))}
              </p>
            )}
            <p
              role={
                inspection.foreign_processes.length > 0 ? "alert" : undefined
              }
            >
              {inspection.foreign_scan_complete
                ? inspection.foreign_processes.length === 0
                  ? "No unmanaged process is working inside the project."
                  : `${inspection.foreign_processes.length} unmanaged process(es) work inside the project: ${inspection.foreign_processes.map((process) => `${process.command} (${process.pid})`).join(", ")}. Stop them yourself before treating the previous writer as released.`
                : "The process scan did not complete; treat concurrent writers as unknown."}
            </p>
            {inspection.record?.text && (
              <details>
                <summary>Handoff text</summary>
                <pre
                  className="max-h-64 overflow-auto whitespace-pre-wrap"
                  dir="auto"
                >
                  {inspection.record.text}
                </pre>
              </details>
            )}
            {inspection.record &&
              !inspection.record.previous_writer_released_at && (
                <div className="space-y-2 rounded border border-warning/40 p-2">
                  <label className="flex items-start gap-2">
                    <input
                      checked={released}
                      onChange={(event) => setReleased(event.target.checked)}
                      type="checkbox"
                    />
                    <span>
                      I verified that the previous writer has stopped and
                      released this project. This is my statement, not an
                      automated check.
                    </span>
                  </label>
                  <Button
                    disabled={!released || acknowledge.isPending}
                    onClick={() => acknowledge.mutate()}
                    type="button"
                  >
                    Record previous writer release
                  </Button>
                </div>
              )}
            {inspection.record?.previous_writer_released_at && (
              <p>
                Previous writer release recorded{" "}
                {new Date(
                  inspection.record.previous_writer_released_at * 1000,
                ).toLocaleString()}
                .
              </p>
            )}
          </div>
        )}
        <form
          className="space-y-2"
          onSubmit={(event) => {
            event.preventDefault();
            importSummary.mutate();
          }}
        >
          <p className="font-medium">Import a summary from another session</p>
          <label className="block space-y-1" htmlFor="handoff-import-label">
            <span>Label (for example the export file name)</span>
            <Input
              id="handoff-import-label"
              onChange={(event) => setLabel(event.target.value)}
              value={label}
            />
          </label>
          <label className="block space-y-1" htmlFor="handoff-import-text">
            <span>Summary text</span>
            <textarea
              className="block w-full rounded border bg-background p-2"
              dir="auto"
              id="handoff-import-text"
              onChange={(event) => setSummary(event.target.value)}
              rows={4}
              value={summary}
            />
          </label>
          <Button
            disabled={importSummary.isPending}
            type="submit"
            variant="outline"
          >
            Store imported summary
          </Button>
        </form>
      </section>
      <section aria-label="Changes" className="space-y-2 rounded border p-3">
        <h3 className="font-semibold">Changes on disk</h3>
        <p>
          Working-tree changes read by the isolated checker, cross-referenced
          with the task board. Delivered, reviewed and accepted are distinct
          states; Cortex Max never commits, merges, pushes or deploys on your
          behalf.
        </p>
        <Button
          disabled={changes.isPending}
          onClick={() => changes.mutate()}
          type="button"
          variant="outline"
        >
          {changes.isPending ? "Reading changes…" : "Review changes"}
        </Button>
        {review && !changes.isError && (
          <div className="space-y-2" data-testid="changes-result">
            <p role="status">
              {review.changes.repository
                ? `${review.changes.changed_paths.length} changed path(s)${review.changes.paths_truncated ? " (list truncated)" : ""}; ${review.awaiting.length} task(s) awaiting review or acceptance.`
                : "The project is not a Git repository; only the task board records changes."}
            </p>
            {review.overlaps.length > 0 && (
              <p role="alert">
                Overlapping outputs:{" "}
                {review.overlaps
                  .map(
                    (overlap) =>
                      `${overlap.path} (${overlap.task_ids.join(", ")})`,
                  )
                  .join("; ")}
                . Coordinate these tasks before accepting either.
              </p>
            )}
            {review.changes.changed_paths.length > 0 && (
              <ul
                aria-label="Changed paths"
                className="max-h-48 overflow-auto text-xs"
              >
                {review.changes.changed_paths.map((entry) => (
                  <li key={`${entry.status}:${entry.path}`}>
                    <code>
                      <bdi dir="ltr">
                        {entry.status} {entry.path}
                      </bdi>
                    </code>
                  </li>
                ))}
              </ul>
            )}
            {review.changes.diff_stat && (
              <pre className="max-h-40 overflow-auto text-xs" dir="ltr">
                {review.changes.diff_stat}
              </pre>
            )}
            {review.changes.diff && (
              <details>
                <summary>
                  Unified diff
                  {review.changes.diff_truncated ? " (truncated)" : ""}
                </summary>
                <pre className="max-h-96 overflow-auto text-xs" dir="ltr">
                  {review.changes.diff}
                </pre>
              </details>
            )}
            {review.edit_receipts.length > 0 && (
              <details>
                <summary>
                  Native edit receipts ({review.edit_receipts.length})
                </summary>
                <ul className="text-xs">
                  {review.edit_receipts.map((receipt) => (
                    <li key={receipt.operation_id}>
                      {receipt.state}:{" "}
                      <bdi dir="ltr">{receipt.relative_path}</bdi>{" "}
                      {receipt.after_sha256.slice(0, 12)}…
                    </li>
                  ))}
                </ul>
              </details>
            )}
            <p className="text-xs text-muted-foreground">
              Check results are what the assignee reported as outputs of kind
              `check` and what the reviewer wrote; Cortex Max did not capture
              their command output. Merged and deployed: not performed by Cortex
              Max.
            </p>
          </div>
        )}
      </section>
    </section>
  );
}
