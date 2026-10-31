import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Folder } from "lucide-react";
import { useState } from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import { ChannelProjectApprovals } from "./ChannelProjectApprovals";
import { ChannelProjectReview } from "./ChannelProjectReview";
import { ChannelProjectTasks } from "./ChannelProjectTasks";
import { ChannelProjectTeam } from "./ChannelProjectTeam";
import { ProjectEditRecovery } from "./ProjectEditRecovery";
import {
  getChannelProjectTeam,
  type ProjectTeamStatus,
} from "@/shared/api/channelProjectTeam";
import { invokeTauri } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/shared/ui/dialog";

type Binding = { channel_id: string; generation: number; root: string | null };
type Status = {
  pending_provider_attempts: { id: string; agent_pubkey: string }[];
  checked_at: number;
  binding: Binding | null;
  folder_readable: boolean;
  folder_error: string | null;
  execution_ready: boolean;
  execution_blocker: string;
};

export function ChannelWorkspaceDialog({
  channelId,
  owner,
}: {
  channelId: string;
  owner: string;
}) {
  const { activeCommunity } = useCommunities();
  if (!activeCommunity) return null;
  return (
    <ScopedWorkspaceDialog
      channelId={channelId}
      key={`${activeCommunity.relayUrl}:${owner}:${channelId}`}
      owner={owner}
      relay={activeCommunity.relayUrl}
    />
  );
}

function ScopedWorkspaceDialog({
  channelId,
  owner,
  relay,
}: {
  channelId: string;
  owner: string;
  relay: string;
}) {
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [releasedAttempt, setReleasedAttempt] = useState<string | null>(null);
  const client = useQueryClient();
  const queryKey = ["channel-workspace", relay, owner, channelId];
  const scope = { channelId, expectedOwner: owner, expectedRelay: relay };
  const query = useQuery({
    queryKey,
    enabled: open,
    queryFn: () => invokeTauri<Status>("get_channel_workspace", scope),
    retry: false,
    staleTime: 0,
  });
  const attached = !!query.data?.binding?.root && !query.isError;
  const team = useQuery({
    queryKey: ["channel-project-team", relay, owner, channelId],
    enabled: open && attached,
    queryFn: () => getChannelProjectTeam(scope),
    retry: false,
    staleTime: 0,
  });
  const picker = useMutation({
    mutationFn: () =>
      invokeTauri<string | null>("pick_channel_workspace_folder"),
    onSuccess: (path) => {
      if (path !== null) setSelected(path);
    },
  });
  const save = useMutation({
    mutationFn: (root: string | null) => {
      if (!query.data || query.isError || query.isFetching)
        throw new Error("Refresh the binding before saving.");
      return invokeTauri<Binding>("set_channel_workspace", {
        ...scope,
        expectedGeneration: query.data.binding?.generation ?? 0,
        root,
      });
    },
    onSuccess: async () => {
      setSelected(null);
      await client.invalidateQueries({ queryKey });
    },
  });
  const release = useMutation({
    mutationFn: (attemptId: string) => {
      if (releasedAttempt !== attemptId)
        throw new Error("Confirm independent writer release first.");
      return invokeTauri<void>("confirm_channel_workspace_writer_release", {
        expectedOwner: owner,
        expectedRelay: relay,
        attemptId,
        writerReleased: true,
      });
    },
    onSuccess: async () => {
      setReleasedAttempt(null);
      await client.invalidateQueries({ queryKey });
    },
  });
  const pending = query.data?.pending_provider_attempts ?? [];
  const busy = picker.isPending || save.isPending || release.isPending;
  const canSave =
    !busy &&
    !!query.data &&
    !query.isError &&
    !query.isFetching &&
    pending.length === 0;
  const error = release.error ?? save.error ?? picker.error ?? query.error;
  const readiness = team.data?.readiness;
  const teamStatus: ProjectTeamStatus | undefined = team.data;
  const agentNames: Record<string, string> = Object.fromEntries(
    (teamStatus?.available_agents ?? []).map((agent) => [
      agent.pubkey,
      agent.display_name,
    ]),
  );
  return (
    <Dialog onOpenChange={setOpen} open={open}>
      <DialogTrigger asChild>
        <Button
          aria-label="Channel local project"
          size="sm"
          title="Channel local project"
          variant="outline"
        >
          <Folder aria-hidden="true" />
          Project
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Local project</DialogTitle>
          <DialogDescription>
            One project per channel on this device. Folder paths are not
            published to the community.
          </DialogDescription>
        </DialogHeader>
        <Tabs defaultValue="workspace">
          <TabsList aria-label="Channel project sections">
            <TabsTrigger value="workspace">Workspace</TabsTrigger>
            <TabsTrigger disabled={!attached} value="team">
              Team and permissions
            </TabsTrigger>
            <TabsTrigger disabled={!attached} value="tasks">
              Tasks
            </TabsTrigger>
            <TabsTrigger disabled={!attached} value="approvals">
              Approvals
              {readiness && readiness.pending_approvals > 0
                ? ` (${readiness.pending_approvals})`
                : ""}
            </TabsTrigger>
            <TabsTrigger disabled={!attached} value="review">
              Review
            </TabsTrigger>
            <TabsTrigger value="recovery">Edit recovery</TabsTrigger>
          </TabsList>
          <TabsContent className="space-y-3 text-sm" value="workspace">
            <p role="status">
              {query.isPending
                ? "Loading binding…"
                : query.isError
                  ? "Binding unavailable"
                  : query.data?.binding?.root
                    ? "Project attached"
                    : "Conversation only"}
            </p>
            {query.data?.binding?.root && (
              <p className="break-all">
                <bdi dir="ltr">{query.data.binding.root}</bdi>
              </p>
            )}
            {query.data && !query.isError && (
              <p>
                {query.data.folder_readable
                  ? "Folder readable by this desktop — agent access not verified."
                  : (query.data.folder_error ?? "No local folder attached.")}
              </p>
            )}
            {attached && (
              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
                <dt className="font-medium">Readiness</dt>
                <dd data-testid="workspace-readiness-state">
                  {team.isPending
                    ? "Checking…"
                    : team.isError
                      ? "Unavailable"
                      : (readiness?.state ?? "not reported")}
                </dd>
                <dt className="font-medium">Branch</dt>
                <dd>
                  <bdi dir="ltr">{readiness?.git.branch ?? "—"}</bdi>
                </dd>
                <dt className="font-medium">Revision</dt>
                <dd className="break-all">
                  <bdi dir="ltr">{readiness?.git.revision ?? "—"}</bdi>
                </dd>
                <dt className="font-medium">Uncommitted changes</dt>
                <dd>
                  {readiness?.git.dirty === undefined ||
                  readiness?.git.dirty === null
                    ? "Not verified"
                    : readiness.git.dirty
                      ? "Present — preserved"
                      : "None reported"}
                </dd>
                <dt className="font-medium">Coordinator</dt>
                <dd>
                  {teamStatus?.coordinator_pubkey
                    ? (agentNames[teamStatus.coordinator_pubkey] ??
                      teamStatus.coordinator_pubkey)
                    : "Not chosen"}
                </dd>
                <dt className="font-medium">Team</dt>
                <dd>
                  {teamStatus?.members.length
                    ? teamStatus.members
                        .map(
                          (member) => `${member.display_name} (${member.role})`,
                        )
                        .join(", ")
                    : "Empty"}
                </dd>
                <dt className="font-medium">Running tasks</dt>
                <dd>{readiness?.running_tasks ?? 0}</dd>
              </dl>
            )}
            <p className="rounded border border-warning/40 p-3">
              {query.isFetching
                ? "Checking current execution readiness…"
                : query.isError || !query.data
                  ? "Execution readiness is unknown. Refresh to verify."
                  : query.data.execution_ready
                    ? "Every readiness check passed on this device. This is not proof of a live provider run or completed work."
                    : "Execution is not ready. Resolve the reported blockers before assigning project work."}
            </p>
            {query.data && !query.isError && query.data.execution_blocker && (
              <p>{query.data.execution_blocker}</p>
            )}
            {query.data?.binding?.root && !query.data.execution_ready && (
              <p>
                Only team members whose runtime has a verified enforcement
                profile launch with project authority. Unsupported and custom
                launchers are refused for project work; they are not replaced or
                granted more access. Existing workers are not redirected.
              </p>
            )}
            {query.data && !query.isError && (
              <p>
                Last folder check:{" "}
                {new Date(query.data.checked_at * 1000).toLocaleString()}
              </p>
            )}
            {selected && (
              <p className="break-all">
                Selected folder:
                <br />
                <bdi dir="ltr">{selected}</bdi>
              </p>
            )}
            {error && <p role="alert">{String(error)}</p>}
            {pending.map((attempt) => (
              <div className="space-y-2 rounded border p-3" key={attempt.id}>
                <p>
                  Remote execution outcome unknown. Check the provider
                  externally; this action does not stop a process.
                </p>
                <p className="break-all">
                  <bdi dir="ltr">{attempt.agent_pubkey}</bdi>
                </p>
                <label className="flex items-start gap-2">
                  <input
                    checked={releasedAttempt === attempt.id}
                    disabled={busy}
                    onChange={(event) =>
                      setReleasedAttempt(
                        event.target.checked ? attempt.id : null,
                      )
                    }
                    type="checkbox"
                  />
                  I independently verified this remote writer has stopped.
                </label>
                <Button
                  disabled={
                    busy ||
                    query.isError ||
                    query.isFetching ||
                    releasedAttempt !== attempt.id
                  }
                  onClick={() => release.mutate(attempt.id)}
                  type="button"
                  variant="outline"
                >
                  Record writer release
                </Button>
              </div>
            ))}
            <div className="flex flex-wrap gap-2">
              <Button
                disabled={busy}
                onClick={() => picker.mutate()}
                type="button"
                variant="outline"
              >
                Choose existing folder
              </Button>
              <Button
                disabled={!selected || !canSave}
                onClick={() => {
                  if (selected) save.mutate(selected);
                }}
                type="button"
              >
                Attach selected folder
              </Button>
              <Button
                disabled={!query.data?.binding?.root || !canSave}
                onClick={() => save.mutate(null)}
                type="button"
                variant="outline"
              >
                Detach project
              </Button>
              <Button
                disabled={busy || query.isFetching}
                onClick={() => {
                  save.reset();
                  release.reset();
                  picker.reset();
                  void query.refetch();
                  void team.refetch();
                }}
                type="button"
                variant="outline"
              >
                Refresh
              </Button>
            </div>
          </TabsContent>
          <TabsContent
            className="text-sm data-[state=inactive]:hidden"
            forceMount
            value="team"
          >
            {open && query.data?.binding?.root && (
              <ChannelProjectTeam
                bindingGeneration={query.data.binding.generation}
                key={query.data.binding.generation}
                scope={scope}
                workspaceVerified={!query.isError && !query.isFetching}
              />
            )}
          </TabsContent>
          <TabsContent value="tasks">
            {open && attached && teamStatus && (
              <ChannelProjectTasks scope={scope} status={teamStatus} />
            )}
            {open && attached && !teamStatus && (
              <p>
                {team.isError ? String(team.error) : "Loading project team…"}
              </p>
            )}
          </TabsContent>
          <TabsContent value="approvals">
            {open && attached && (
              <ChannelProjectApprovals
                agentNames={agentNames}
                scope={scope}
                teamGeneration={teamStatus?.generation ?? null}
              />
            )}
          </TabsContent>
          <TabsContent value="review">
            {open && attached && query.data?.binding && (
              <ChannelProjectReview
                bindingGeneration={query.data.binding.generation}
                key={query.data.binding.generation}
                scope={scope}
              />
            )}
          </TabsContent>
          <TabsContent value="recovery">
            <ProjectEditRecovery scope={scope} />
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}
