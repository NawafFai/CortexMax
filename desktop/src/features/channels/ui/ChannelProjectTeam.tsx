import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import {
  addApprovedCheck,
  projectTeamDraft,
  projectTeamSaveError,
  releaseProjectMember,
  removeApprovedCheck,
  selectProjectCoordinator,
} from "@/features/channels/lib/projectTeamDraft";
import {
  DEFAULT_PROJECT_CAPABILITIES,
  DEFAULT_PROJECT_ROLES,
  getChannelProjectTeam,
  setChannelProjectPaused,
  setChannelProjectTeam,
  type ProjectTeamDraft,
  type ProjectTeamScope,
} from "@/shared/api/channelProjectTeam";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { ChannelProjectInspection } from "./ChannelProjectInspection";
import { ChannelProjectReadiness } from "./ChannelProjectReadiness";
import { ProjectCoordinatorStartup } from "./ProjectCoordinatorStartup";
import { ProjectTeamMemberFields } from "./ProjectTeamMemberFields";

export function ChannelProjectTeam({
  scope,
  bindingGeneration,
  workspaceVerified,
}: {
  scope: ProjectTeamScope;
  bindingGeneration: number;
  workspaceVerified: boolean;
}) {
  const client = useQueryClient();
  const [draft, setDraft] = useState<ProjectTeamDraft | null>(null);
  const [candidate, setCandidate] = useState("");
  const [newCheck, setNewCheck] = useState("");
  const [checkError, setCheckError] = useState<string | null>(null);
  const queryKey = [
    "channel-project-team",
    scope.expectedRelay,
    scope.expectedOwner,
    scope.channelId,
  ];
  const query = useQuery({
    queryKey,
    queryFn: () => getChannelProjectTeam(scope),
    retry: false,
    staleTime: 0,
    refetchInterval: draft === null ? 10_000 : false,
  });
  const invalidate = async () =>
    Promise.all([
      client.invalidateQueries({ queryKey }),
      client.invalidateQueries({
        queryKey: [
          "channel-workspace",
          scope.expectedRelay,
          scope.expectedOwner,
          scope.channelId,
        ],
      }),
    ]);
  const save = useMutation({
    mutationFn: async () => {
      if (!draft || !query.data || query.isError || query.isFetching)
        throw new Error("Refresh the project team before saving.");
      const error = projectTeamSaveError(draft, query.data);
      if (error) throw new Error(error);
      return setChannelProjectTeam(scope, draft);
    },
    onSuccess: async (status) => {
      setDraft(null);
      setCandidate("");
      client.setQueryData(queryKey, status);
      await invalidate();
    },
  });
  const pause = useMutation({
    mutationFn: async (paused: boolean) => {
      if (!query.data || query.isError || query.isFetching || draft)
        throw new Error(
          "Save or discard your draft, then refresh, before pausing.",
        );
      return setChannelProjectPaused(scope, query.data.generation, paused);
    },
    onSuccess: async (status) => {
      client.setQueryData(queryKey, status);
      await invalidate();
    },
  });
  const current = draft ?? (query.data ? projectTeamDraft(query.data) : null);
  const validation =
    draft && query.data ? projectTeamSaveError(draft, query.data) : null;
  const bindingChanged =
    !!query.data && query.data.binding_generation !== bindingGeneration;
  const editable =
    !!query.data &&
    !query.isError &&
    !query.isFetching &&
    !bindingChanged &&
    workspaceVerified;
  const roles = query.data?.roles ?? DEFAULT_PROJECT_ROLES;
  const capabilities = query.data?.capabilities ?? DEFAULT_PROJECT_CAPABILITIES;
  const error = save.error ?? pause.error ?? query.error;
  return (
    <section aria-label="Project team" className="space-y-3 border-t pt-3">
      <h3 className="font-semibold">Project team and permissions</h3>
      <p>
        Pick existing agents for this channel only. Releasing a member keeps
        their global identity and conversations. Membership is not proof that a
        worker is running.
      </p>
      {query.isPending && <p>Loading project team…</p>}
      {error && <p role="alert">{String(error)}</p>}
      {bindingChanged && (
        <p role="alert">
          The project folder changed. Refresh the Workspace section before
          editing this team.
        </p>
      )}
      {query.data && current && (
        <>
          <p>
            Saved permission revision: {query.data.generation}.{" "}
            {draft ? "Unsaved changes." : "No unsaved changes."}
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <span>Dispatch is {query.data.paused ? "paused" : "active"}.</span>
            <Button
              disabled={!editable || !!draft || pause.isPending}
              onClick={() => pause.mutate(!query.data?.paused)}
              type="button"
              variant="outline"
            >
              {query.data.paused ? "Resume project work" : "Pause project work"}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            Pausing stops new assignments and task starts; every native tool
            call and gated turn refuses project work on its next authority
            check. It does not kill an in-flight process and never deletes work.
          </p>
          <fieldset
            className="space-y-3"
            disabled={!editable || save.isPending}
          >
            <label className="block space-y-1">
              <span>Project coordinator</span>
              <select
                className="block w-full rounded border bg-background p-2"
                onChange={(event) => {
                  if (query.data)
                    setDraft(
                      selectProjectCoordinator(
                        current,
                        query.data,
                        event.target.value || null,
                      ),
                    );
                }}
                value={current.coordinator_pubkey ?? ""}
              >
                <option value="">No coordinator</option>
                {query.data.available_agents.map((agent) => (
                  <option key={agent.pubkey} value={agent.pubkey}>
                    {agent.display_name}
                  </option>
                ))}
              </select>
            </label>
            <p>
              Becoming coordinator does not grant extra permissions. Select each
              permission below. Publishing, deployment and destructive actions
              are decided per request in the Approvals tab.
            </p>
            <label className="flex items-center gap-2">
              <input
                checked={current.paused}
                onChange={(event) =>
                  setDraft({ ...current, paused: event.target.checked })
                }
                type="checkbox"
              />
              Pause project work
            </label>
            <label className="block space-y-1">
              <span>Existing agent to add</span>
              <select
                className="block w-full rounded border bg-background p-2"
                onChange={(event) => setCandidate(event.target.value)}
                value={candidate}
              >
                <option value="">Choose an agent</option>
                {query.data.available_agents
                  .filter(
                    (agent) =>
                      !current.members.some(
                        (member) => member.pubkey === agent.pubkey,
                      ),
                  )
                  .map((agent) => (
                    <option key={agent.pubkey} value={agent.pubkey}>
                      {agent.display_name}
                    </option>
                  ))}
              </select>
            </label>
            <Button
              disabled={!candidate}
              onClick={() => {
                const agent = query.data?.available_agents.find(
                  (agent) => agent.pubkey === candidate,
                );
                if (!agent) return;
                setDraft({
                  ...current,
                  members: [
                    ...current.members,
                    {
                      pubkey: agent.pubkey,
                      display_name: agent.display_name,
                      role: "member",
                      capabilities: ["read"],
                    },
                  ],
                });
                setCandidate("");
              }}
              type="button"
              variant="outline"
            >
              Add to project team
            </Button>
            {current.members.map((member) => {
              const agent = query.data?.available_agents.find(
                (agent) => agent.pubkey === member.pubkey,
              );
              return (
                <ProjectTeamMemberFields
                  capabilities={capabilities}
                  enforcement={agent?.enforcement}
                  key={member.pubkey}
                  member={member}
                  onChange={(updated) =>
                    setDraft({
                      ...current,
                      members: current.members.map((item) =>
                        item.pubkey === updated.pubkey ? updated : item,
                      ),
                    })
                  }
                  onRelease={() =>
                    setDraft(releaseProjectMember(current, member.pubkey))
                  }
                  roles={roles}
                  runtime={agent?.runtime_id ?? null}
                />
              );
            })}
            <section
              aria-label="Approved checks"
              className="space-y-2 rounded border p-3"
            >
              <h4 className="font-medium">Approved check commands</h4>
              <p className="text-xs text-muted-foreground">
                Exact command lines that members with the run_checks grant may
                run inside the project. Anything else is refused and queued for
                your decision.
              </p>
              <ul className="space-y-1">
                {current.approved_checks.map((command) => (
                  <li className="flex items-center gap-2" key={command}>
                    <code className="break-all">
                      <bdi dir="ltr">{command}</bdi>
                    </code>
                    <Button
                      onClick={() =>
                        setDraft(removeApprovedCheck(current, command))
                      }
                      type="button"
                      variant="outline"
                    >
                      Remove
                    </Button>
                  </li>
                ))}
              </ul>
              <div className="flex flex-wrap items-center gap-2">
                <Input
                  aria-label="New approved check"
                  dir="ltr"
                  onChange={(event) => {
                    setNewCheck(event.target.value);
                    setCheckError(null);
                  }}
                  placeholder="cargo test --offline"
                  value={newCheck}
                />
                <Button
                  disabled={!newCheck.trim()}
                  onClick={() => {
                    try {
                      setDraft(addApprovedCheck(current, newCheck));
                      setNewCheck("");
                      setCheckError(null);
                    } catch (failure) {
                      setCheckError(String(failure));
                    }
                  }}
                  type="button"
                  variant="outline"
                >
                  Add approved check
                </Button>
              </div>
              {checkError && <p role="alert">{checkError}</p>}
            </section>
          </fieldset>
          {validation && <p role="alert">{validation}</p>}
          {query.data.execution_blockers.map((blocker) => (
            <p className="rounded border border-warning/40 p-2" key={blocker}>
              {blocker}
            </p>
          ))}
        </>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          disabled={!!draft || !editable || save.isPending}
          onClick={() => {
            if (query.data) setDraft(projectTeamDraft(query.data));
          }}
          type="button"
          variant="outline"
        >
          Review and re-save grants
        </Button>
        <Button
          disabled={!draft || !editable || save.isPending || !!validation}
          onClick={() => save.mutate()}
          type="button"
        >
          {save.isPending ? "Saving permissions…" : "Save project permissions"}
        </Button>
        <Button
          disabled={save.isPending || query.isFetching}
          onClick={() => {
            save.reset();
            pause.reset();
            void query.refetch();
          }}
          type="button"
          variant="outline"
        >
          Refresh team
        </Button>
        <Button
          disabled={!draft || save.isPending || query.isFetching}
          onClick={() => {
            setDraft(null);
            setCandidate("");
            save.reset();
          }}
          type="button"
          variant="outline"
        >
          Discard team changes
        </Button>
      </div>
      {query.data && !query.isError && (
        <>
          <ChannelProjectReadiness readiness={query.data.readiness} />
          <ProjectCoordinatorStartup
            bindingGeneration={bindingGeneration}
            coordinatorPubkey={query.data.coordinator_pubkey}
            disabled={
              !!draft || !editable || save.isPending || query.data.paused
            }
            key={`${scope.expectedOwner}:${scope.expectedRelay}:${scope.channelId}:${bindingGeneration}:${query.data.generation}:${draft !== null}:${workspaceVerified}:${query.data.paused}`}
            scope={scope}
            teamGeneration={query.data.generation}
          />
          <ChannelProjectInspection
            bindingGeneration={bindingGeneration}
            disabled={!!draft || !editable || save.isPending}
            key={`${query.data.generation}:${draft !== null}:${workspaceVerified}`}
            scope={scope}
            teamGeneration={query.data.generation}
          />
        </>
      )}
    </section>
  );
}
