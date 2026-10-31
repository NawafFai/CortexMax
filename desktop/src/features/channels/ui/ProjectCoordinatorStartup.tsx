import { useMutation } from "@tanstack/react-query";
import { useState } from "react";

import {
  startChannelProjectCoordinator,
  type ProjectTeamScope,
} from "@/shared/api/channelProjectTeam";
import { Button } from "@/shared/ui/button";

export function ProjectCoordinatorStartup({
  scope,
  bindingGeneration,
  teamGeneration,
  coordinatorPubkey,
  disabled,
}: {
  scope: ProjectTeamScope;
  bindingGeneration: number;
  teamGeneration: number;
  coordinatorPubkey: string | null;
  disabled: boolean;
}) {
  const [explicitBridgeOptIn, setExplicitBridgeOptIn] = useState(false);
  const startup = useMutation({
    retry: false,
    mutationFn: async () => {
      if (disabled || !coordinatorPubkey)
        throw new Error("Save and unpause a project coordinator first.");
      const result = await startChannelProjectCoordinator(
        scope,
        bindingGeneration,
        teamGeneration,
        coordinatorPubkey,
        explicitBridgeOptIn,
      );
      if (
        result.binding_generation !== bindingGeneration ||
        result.team_generation !== teamGeneration ||
        result.coordinator_pubkey !== coordinatorPubkey ||
        result.execution_ready !== false ||
        (result.status !== "started" && result.status !== "blocked") ||
        (result.status === "started" && !result.runtime_receipt)
      )
        throw new Error(
          "Startup receipt is invalid or stale. Refresh before taking further action.",
        );
      return result;
    },
  });
  const result = startup.data;
  return (
    <section
      aria-label="Coordinator startup"
      className="space-y-2 rounded border p-3"
    >
      <h4 className="font-medium">Start the saved project coordinator</h4>
      <p>
        Starts an existing locally managed agent only. This does not recreate
        your team, copy sign-in credentials or certify project completion.
      </p>
      {result?.requires_explicit_opt_in && result.status === "blocked" && (
        <label className="flex items-start gap-2">
          <input
            checked={explicitBridgeOptIn}
            disabled={disabled || startup.isPending}
            onChange={(event) => setExplicitBridgeOptIn(event.target.checked)}
            type="checkbox"
          />
          <span>
            Allow the isolated Codex project bridge for this start. The saved
            identity and runtime are preserved; shell tools stay disabled and
            project access uses the saved permissions. A separate scoped sign-in
            is required; your existing login is not copied.
          </span>
        </label>
      )}
      <Button
        disabled={
          disabled ||
          !coordinatorPubkey ||
          startup.isPending ||
          result?.status === "started"
        }
        onClick={() => startup.mutate()}
        type="button"
      >
        {startup.isPending
          ? "Starting coordinator…"
          : "Start project coordinator"}
      </Button>
      {disabled && <p>Save current permissions and unpause before starting.</p>}
      {startup.error && (
        <p role="alert">
          {String(startup.error)} Startup has not been confirmed. Inspect the
          agent's runtime before retrying; no automatic retry was made.
        </p>
      )}
      {result && !startup.isError && (
        <div aria-live="polite">
          {result.blockers.map((blocker) => (
            <p key={blocker.code}>{blocker.message}</p>
          ))}
          {result.authentication_required && (
            <p>
              Owner-approved sign-in to the separate project scope is still
              required. No live account was tested.
            </p>
          )}
          {result.status === "started" && (
            <p>
              Coordinator process started. This is not proof that the agent has
              answered, accessed the project or completed a task. Verify a
              bounded task and its evidence before treating execution as ready.
            </p>
          )}
        </div>
      )}
    </section>
  );
}
