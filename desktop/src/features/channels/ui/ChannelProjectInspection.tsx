import { useMutation } from "@tanstack/react-query";

import { invokeTauri } from "@/shared/api/tauri";
import type { ProjectTeamScope } from "@/shared/api/channelProjectTeam";
import { Button } from "@/shared/ui/button";

type Inspection = {
  channel_id: string;
  binding_generation: number;
  team_generation: number;
  checked_at: number;
  evidence: {
    actual_cwd: string;
    revision: string | null;
    branch: string | null;
    dirty: boolean | null;
    runtime: string;
  };
};

export function ChannelProjectInspection({
  scope,
  bindingGeneration,
  teamGeneration,
  disabled,
}: {
  scope: ProjectTeamScope;
  bindingGeneration: number;
  teamGeneration: number;
  disabled: boolean;
}) {
  const inspection = useMutation({
    mutationFn: async () => {
      const result = await invokeTauri<Inspection>("inspect_channel_project", {
        ...scope,
        expectedBindingGeneration: bindingGeneration,
        expectedTeamGeneration: teamGeneration,
      });
      if (
        result.channel_id !== scope.channelId ||
        result.binding_generation !== bindingGeneration ||
        result.team_generation !== teamGeneration
      )
        throw new Error(
          "Project or permissions changed. Run a new inspection.",
        );
      return result;
    },
  });
  const evidence = inspection.data?.evidence;
  return (
    <section
      aria-label="Local project inspection"
      className="space-y-2 rounded border p-3"
    >
      <h4 className="font-medium">Isolated local inspection</h4>
      <p>
        Tests the saved coordinator's read and check permissions in a restricted
        local process. This does not start an AI agent, verify model sign-in or
        prove task completion. Results are a point-in-time snapshot.
      </p>
      <Button
        disabled={disabled || inspection.isPending}
        onClick={() => {
          inspection.reset();
          inspection.mutate();
        }}
        type="button"
        variant="outline"
      >
        {inspection.isPending
          ? "Inspecting project…"
          : "Run isolated local inspection"}
      </Button>
      {disabled && (
        <p>Save current permissions and refresh before inspecting.</p>
      )}
      {inspection.error && <p role="alert">{String(inspection.error)}</p>}
      {evidence && !inspection.isError && (
        <dl className="space-y-1">
          <div>
            <dt className="font-medium">Actual working directory</dt>
            <dd className="break-all">
              <bdi dir="ltr">{evidence.actual_cwd}</bdi>
            </dd>
          </div>
          <div>
            <dt className="font-medium">Branch</dt>
            <dd>
              <bdi dir="ltr">{evidence.branch ?? "Not a Git branch"}</bdi>
            </dd>
          </div>
          <div>
            <dt className="font-medium">Revision</dt>
            <dd className="break-all">
              <bdi dir="ltr">
                {evidence.revision ?? "No Git revision reported"}
              </bdi>
            </dd>
          </div>
          <div>
            <dt className="font-medium">Uncommitted changes</dt>
            <dd>
              {evidence.dirty === null
                ? "Not reported"
                : evidence.dirty
                  ? "Present — preserved"
                  : "None reported"}
            </dd>
          </div>
          <div>
            <dt className="font-medium">Checker</dt>
            <dd>
              <bdi dir="ltr">{evidence.runtime}</bdi>
            </dd>
          </div>
          <div>
            <dt className="font-medium">Checked at</dt>
            <dd>
              {new Date(
                (inspection.data?.checked_at ?? 0) * 1000,
              ).toLocaleString()}
            </dd>
          </div>
        </dl>
      )}
    </section>
  );
}
