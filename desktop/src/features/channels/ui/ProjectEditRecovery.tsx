import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import type { ProjectTeamScope } from "@/shared/api/channelProjectTeam";
import { invokeTauri } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

type PendingEdit = {
  operation_id: string;
  channel_id: string;
  relative_path: string;
  before_sha256: string;
  after_sha256: string;
};

export function ProjectEditRecovery({ scope }: { scope: ProjectTeamScope }) {
  const client = useQueryClient();
  const [selected, setSelected] = useState<string | null>(null);
  const [observedHash, setObservedHash] = useState("");
  const [writerStopped, setWriterStopped] = useState(false);
  const queryKey = [
    "pending-project-edits",
    scope.expectedRelay,
    scope.expectedOwner,
  ];
  const query = useQuery({
    queryKey,
    queryFn: () =>
      invokeTauri<PendingEdit[]>("get_pending_project_edits", {
        expectedOwner: scope.expectedOwner,
        expectedRelay: scope.expectedRelay,
      }),
    staleTime: 0,
    retry: false,
  });
  const reconcile = useMutation({
    mutationFn: () => {
      const hash = observedHash.trim().toLowerCase();
      if (
        !selected ||
        !writerStopped ||
        query.isError ||
        query.isFetching ||
        !query.data?.some((edit) => edit.operation_id === selected) ||
        !(hash === "absent" || /^[a-f0-9]{64}$/.test(hash))
      )
        throw new Error(
          "Verify the stopped writer and current file hash before reconciling.",
        );
      return invokeTauri<void>("reconcile_project_edit", {
        expectedOwner: scope.expectedOwner,
        expectedRelay: scope.expectedRelay,
        operationId: selected,
        expectedCurrentSha256: hash,
        confirmedWriterStopped: true,
      });
    },
    onSuccess: async () => {
      setSelected(null);
      setObservedHash("");
      setWriterStopped(false);
      await Promise.all([
        client.invalidateQueries({ queryKey }),
        client.invalidateQueries({
          queryKey: [
            "channel-workspace",
            scope.expectedRelay,
            scope.expectedOwner,
          ],
        }),
        client.invalidateQueries({
          queryKey: [
            "channel-project-team",
            scope.expectedRelay,
            scope.expectedOwner,
          ],
        }),
      ]);
    },
  });
  const pending = query.data ?? [];
  const available = !query.isError && !query.isFetching && !reconcile.isPending;
  const validHash =
    observedHash.trim().toLowerCase() === "absent" ||
    /^[a-f0-9]{64}$/i.test(observedHash.trim());
  const error = reconcile.error ?? query.error;
  return (
    <section aria-label="Project edit recovery" className="space-y-3 text-sm">
      <h3 className="font-semibold">Owner-only edit recovery</h3>
      <p>
        A missing receipt is not success or failure. These edits may block new
        changes across this community. Inspect the original channel and file,
        and independently stop its writer before resolving an uncertain outcome.
      </p>
      <p>
        Recovery records what you verified. It never rewrites files, retries an
        edit, deletes evidence or accepts a delivery as reviewed.
      </p>
      {query.isPending && <p>Loading uncertain edits…</p>}
      {error && <p role="alert">{String(error)}</p>}
      {query.isSuccess && pending.length === 0 && (
        <p>No unresolved native edit receipts.</p>
      )}
      {pending.map((edit) => (
        <fieldset
          className="space-y-2 rounded border p-3"
          key={edit.operation_id}
          disabled={!available}
        >
          <legend className="px-1 break-all">{edit.relative_path}</legend>
          <p className="break-all">
            Original channel:
            <br />
            <bdi dir="ltr">{edit.channel_id}</bdi>
          </p>
          <p className="break-all">
            Operation:
            <br />
            <bdi dir="ltr">{edit.operation_id}</bdi>
          </p>
          <p className="break-all">
            Expected previous hash:
            <br />
            <bdi dir="ltr">{edit.before_sha256}</bdi>
          </p>
          <p className="break-all">
            Intended new hash:
            <br />
            <bdi dir="ltr">{edit.after_sha256}</bdi>
          </p>
          <Button
            onClick={() => {
              setSelected(edit.operation_id);
              setObservedHash("");
              setWriterStopped(false);
              reconcile.reset();
            }}
            type="button"
            variant="outline"
          >
            Inspect and resolve this receipt
          </Button>
          {selected === edit.operation_id && (
            <div className="space-y-2">
              <label
                className="block space-y-1"
                htmlFor="project-edit-observed-hash"
              >
                <span>Current file SHA-256, independently inspected</span>
                <Input
                  dir="ltr"
                  id="project-edit-observed-hash"
                  onChange={(event) => setObservedHash(event.target.value)}
                  placeholder="64 hexadecimal characters, or absent"
                  value={observedHash}
                />
              </label>
              <p>
                Enter absent only if the original file is now missing. Copying
                an intended hash is not independent verification.
              </p>
              <label className="flex items-start gap-2">
                <input
                  checked={writerStopped}
                  onChange={(event) => setWriterStopped(event.target.checked)}
                  type="checkbox"
                />
                I independently confirmed this original writer and its helpers
                have stopped.
              </label>
              <Button
                disabled={!available || !validHash || !writerStopped}
                onClick={() => reconcile.mutate()}
                type="button"
              >
                Record inspected outcome
              </Button>
            </div>
          )}
        </fieldset>
      ))}
      <Button
        disabled={query.isFetching || reconcile.isPending}
        onClick={() => {
          reconcile.reset();
          void query.refetch();
        }}
        type="button"
        variant="outline"
      >
        Refresh edit receipts
      </Button>
    </section>
  );
}
