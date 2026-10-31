import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import {
  describeDispatch,
  describeReadiness,
  groupTasks,
  ownerActionsFor,
  reportLines,
} from "@/features/channels/lib/projectTaskBoard";
import {
  createChannelProjectTask,
  dispatchChannelProjectTasks,
  listChannelProjectTasks,
  resolveChannelProjectTaskDispatch,
  transitionChannelProjectTask,
  type OwnerTaskAction,
  type ProjectTask,
  type ProjectTeamScope,
  type ProjectTeamStatus,
} from "@/shared/api/channelProjectTeam";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

function ReportBlock({ label, text }: { label: string; text: string }) {
  const seen = new Map<string, number>();
  const lines = reportLines(text).map((line) => {
    const occurrence = (seen.get(line.text) ?? 0) + 1;
    seen.set(line.text, occurrence);
    return { ...line, key: `${line.text}#${occurrence}` };
  });
  return (
    <div>
      <dt className="font-medium">{label}</dt>
      <dd>
        {lines.map((line) => (
          <p dir={line.dir} key={line.key}>
            {line.text}
          </p>
        ))}
      </dd>
    </div>
  );
}

export function ChannelProjectTasks({
  scope,
  status,
}: {
  scope: ProjectTeamScope;
  status: ProjectTeamStatus;
}) {
  const client = useQueryClient();
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [assignee, setAssignee] = useState<string>(
    status.coordinator_pubkey ?? "",
  );
  const [blockers, setBlockers] = useState<Record<string, string>>({});
  const [reassign, setReassign] = useState<Record<string, string>>({});
  const [observed, setObserved] = useState<Record<string, string>>({});
  const names: Record<string, string> = Object.fromEntries(
    status.available_agents.map((agent) => [agent.pubkey, agent.display_name]),
  );
  const readiness = status.readiness
    ? describeReadiness(status.readiness.state)
    : { label: "Readiness unknown", canStartTask: false };
  const queryKey = [
    "channel-project-tasks",
    scope.expectedRelay,
    scope.expectedOwner,
    scope.channelId,
  ];
  const teamKey = [
    "channel-project-team",
    scope.expectedRelay,
    scope.expectedOwner,
    scope.channelId,
  ];
  const query = useQuery({
    queryKey,
    queryFn: () => listChannelProjectTasks(scope),
    retry: false,
    staleTime: 0,
    refetchInterval: 5_000,
  });
  const afterChange = async () => {
    // Publish pending notifications now rather than waiting for the next tick;
    // the dispatcher itself stays bounded and idempotent.
    try {
      await dispatchChannelProjectTasks(scope);
    } catch {
      // The background loop reconciles; the board shows the dispatch state.
    }
    await Promise.all([
      client.invalidateQueries({ queryKey }),
      client.invalidateQueries({ queryKey: teamKey }),
    ]);
  };
  const create = useMutation({
    mutationFn: () => {
      if (!title.trim() || !description.trim())
        throw new Error("Give the task a title and a bounded description.");
      if (assignee && !readiness.canStartTask)
        throw new Error(
          `Execution is not authorized (${readiness.label}). Resolve the readiness checks or plan the task without assigning it.`,
        );
      return createChannelProjectTask(scope, {
        title: title.trim(),
        description: description.trim(),
        assigneePubkey: assignee || null,
        returnTo: "owner",
      });
    },
    onSuccess: async () => {
      setTitle("");
      setDescription("");
      await afterChange();
    },
  });
  const act = useMutation({
    mutationFn: (input: { task: ProjectTask; action: OwnerTaskAction }) => {
      if (input.action.action === "assign" && !readiness.canStartTask)
        throw new Error(
          `Execution is not authorized (${readiness.label}); assignment is refused.`,
        );
      return transitionChannelProjectTask(
        scope,
        input.task.task_id,
        input.task.revision,
        input.action,
      );
    },
    onSuccess: afterChange,
  });
  const resolve = useMutation({
    mutationFn: (input: {
      task: ProjectTask;
      observedEventId: string | null;
    }) =>
      resolveChannelProjectTaskDispatch(
        scope,
        input.task.task_id,
        input.task.revision,
        input.observedEventId,
      ),
    onSuccess: afterChange,
  });
  const columns = groupTasks(query.data ?? []);
  const error = create.error ?? act.error ?? resolve.error ?? query.error;
  return (
    <section aria-label="Project task board" className="space-y-3 text-sm">
      <h3 className="font-semibold">Task board</h3>
      <p role="status">{readiness.label}.</p>
      <p>
        Tasks advance only through recorded transitions: an assignee delivers
        with evidence, an independent reviewer files a verdict, and you accept.
        A final summary can only be filed after an approved review. Chat text
        never changes a task.
      </p>
      {error && <p role="alert">{String(error)}</p>}
      <form
        className="space-y-2 rounded border p-3"
        onSubmit={(event) => {
          event.preventDefault();
          create.mutate();
        }}
      >
        <h4 className="font-medium">Plan one bounded task</h4>
        <label className="block space-y-1" htmlFor="project-task-title">
          <span>Title</span>
          <Input
            id="project-task-title"
            onChange={(event) => setTitle(event.target.value)}
            value={title}
          />
        </label>
        <label className="block space-y-1">
          <span>What exactly should be delivered</span>
          <textarea
            className="block w-full rounded border bg-background p-2"
            dir="auto"
            onChange={(event) => setDescription(event.target.value)}
            rows={3}
            value={description}
          />
        </label>
        <label className="block space-y-1">
          <span>Assign to</span>
          <select
            className="block w-full rounded border bg-background p-2"
            onChange={(event) => setAssignee(event.target.value)}
            value={assignee}
          >
            <option value="">Plan only (assign later)</option>
            {status.members.map((member) => (
              <option key={member.pubkey} value={member.pubkey}>
                {member.display_name} ({member.role})
              </option>
            ))}
          </select>
        </label>
        <Button disabled={create.isPending} type="submit">
          {assignee ? "Plan and assign task" : "Plan task"}
        </Button>
      </form>
      {query.isPending && <p>Loading task board…</p>}
      <div className="space-y-3">
        {columns.map((column) => (
          <section
            aria-label={column.label}
            className="space-y-2"
            key={column.id}
          >
            <h4 className="font-medium">
              {column.label} ({column.tasks.length})
            </h4>
            {column.tasks.map((task) => (
              <article
                className="space-y-2 rounded border p-3"
                data-task-state={task.state}
                key={task.task_id}
              >
                <p className="font-medium">
                  {task.title} — {task.state} (revision {task.revision})
                </p>
                <p dir="auto">{task.description}</p>
                <p className="text-xs text-muted-foreground">
                  Assignee:{" "}
                  {task.assignee_pubkey
                    ? (names[task.assignee_pubkey] ?? task.assignee_pubkey)
                    : "—"}{" "}
                  · Return to:{" "}
                  {task.return_to === "owner"
                    ? "owner"
                    : (names[task.return_to] ?? task.return_to)}{" "}
                  · <bdi dir="ltr">{task.task_id}</bdi>
                </p>
                {task.required_inputs.length > 0 && (
                  <p>Waiting for: {task.required_inputs.join("; ")}</p>
                )}
                {task.blocker && <p role="alert">Blocked: {task.blocker}</p>}
                {task.outputs.length > 0 && (
                  <ul aria-label="Delivered outputs" className="text-xs">
                    {task.outputs.map((output) => (
                      <li key={`${output.kind}:${output.reference}`}>
                        {output.kind}: <bdi dir="ltr">{output.reference}</bdi>
                        {output.sha256
                          ? ` (${output.sha256.slice(0, 12)}…)`
                          : ""}
                      </li>
                    ))}
                  </ul>
                )}
                {task.review && (
                  <p>
                    Independent review by{" "}
                    {names[task.review.reviewer_pubkey] ??
                      task.review.reviewer_pubkey}
                    : {task.review.verdict} — {task.review.notes}
                  </p>
                )}
                {task.report && (
                  <section
                    aria-label="Final report"
                    className="space-y-1 rounded border p-2"
                    dir="rtl"
                  >
                    <dl>
                      <ReportBlock
                        label="ما طُلب"
                        text={task.report.requested}
                      />
                      <ReportBlock
                        label="ما سُلّم فعلًا"
                        text={task.report.delivered}
                      />
                      <ReportBlock
                        label="ما تحقق منه"
                        text={task.report.verified}
                      />
                      <ReportBlock
                        label="ما فشل أو بقي غير محقق"
                        text={task.report.failed_or_unverified}
                      />
                      <ReportBlock
                        label="القرار أو الموافقة المطلوبة"
                        text={task.report.decision_needed}
                      />
                    </dl>
                  </section>
                )}
                <p className="text-xs text-muted-foreground">
                  {describeDispatch(task)}
                </p>
                {task.dispatch_state === "uncertain" && (
                  <div className="flex flex-wrap items-center gap-2">
                    <Input
                      aria-label="Observed notification event id"
                      onChange={(event) =>
                        setObserved({
                          ...observed,
                          [task.task_id]: event.target.value,
                        })
                      }
                      placeholder="Event id seen in the channel"
                      value={observed[task.task_id] ?? ""}
                    />
                    <Button
                      disabled={
                        resolve.isPending || !observed[task.task_id]?.trim()
                      }
                      onClick={() =>
                        resolve.mutate({
                          task,
                          observedEventId: observed[task.task_id].trim(),
                        })
                      }
                      type="button"
                      variant="outline"
                    >
                      It arrived
                    </Button>
                    <Button
                      disabled={resolve.isPending}
                      onClick={() =>
                        resolve.mutate({ task, observedEventId: null })
                      }
                      type="button"
                      variant="outline"
                    >
                      Not delivered, try once more
                    </Button>
                  </div>
                )}
                <div className="flex flex-wrap items-center gap-2">
                  {ownerActionsFor(task).map((action) => {
                    if (action === "assign")
                      return (
                        <span className="flex items-center gap-1" key={action}>
                          <select
                            aria-label="Assignee"
                            className="rounded border bg-background p-1"
                            onChange={(event) =>
                              setReassign({
                                ...reassign,
                                [task.task_id]: event.target.value,
                              })
                            }
                            value={
                              reassign[task.task_id] ??
                              status.coordinator_pubkey ??
                              ""
                            }
                          >
                            {status.members.map((member) => (
                              <option key={member.pubkey} value={member.pubkey}>
                                {member.display_name}
                              </option>
                            ))}
                          </select>
                          <Button
                            disabled={act.isPending || !readiness.canStartTask}
                            onClick={() =>
                              act.mutate({
                                task,
                                action: {
                                  action: "assign",
                                  assignee_pubkey:
                                    reassign[task.task_id] ??
                                    status.coordinator_pubkey ??
                                    "",
                                },
                              })
                            }
                            type="button"
                          >
                            Assign
                          </Button>
                        </span>
                      );
                    if (action === "block")
                      return (
                        <span className="flex items-center gap-1" key={action}>
                          <Input
                            aria-label="Blocker"
                            onChange={(event) =>
                              setBlockers({
                                ...blockers,
                                [task.task_id]: event.target.value,
                              })
                            }
                            placeholder="Blocker"
                            value={blockers[task.task_id] ?? ""}
                          />
                          <Button
                            disabled={
                              act.isPending || !blockers[task.task_id]?.trim()
                            }
                            onClick={() =>
                              act.mutate({
                                task,
                                action: {
                                  action: "block",
                                  blocker: blockers[task.task_id].trim(),
                                },
                              })
                            }
                            type="button"
                            variant="outline"
                          >
                            Block
                          </Button>
                        </span>
                      );
                    return (
                      <Button
                        disabled={act.isPending}
                        key={action}
                        onClick={() => act.mutate({ task, action: { action } })}
                        type="button"
                        variant={action === "accept" ? "default" : "outline"}
                      >
                        {action === "provide_input"
                          ? "Inputs provided"
                          : action === "accept"
                            ? "Accept delivery"
                            : action[0].toUpperCase() + action.slice(1)}
                      </Button>
                    );
                  })}
                </div>
              </article>
            ))}
          </section>
        ))}
      </div>
      <Button
        disabled={query.isFetching}
        onClick={() => void query.refetch()}
        type="button"
        variant="outline"
      >
        Refresh board
      </Button>
    </section>
  );
}
