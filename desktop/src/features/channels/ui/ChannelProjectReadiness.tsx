import { describeReadiness } from "@/features/channels/lib/projectTaskBoard";
import type { ProjectReadiness } from "@/shared/api/channelProjectTeam";

function statusLabel(status: string) {
  switch (status) {
    case "ok":
      return "OK";
    case "warn":
      return "Warning";
    case "fail":
      return "Failed";
    default:
      return "Unknown";
  }
}

export function ChannelProjectReadiness({
  readiness,
}: {
  readiness: ProjectReadiness | undefined;
}) {
  if (!readiness) {
    return (
      <section
        aria-label="Project readiness"
        className="space-y-2 rounded border p-3"
      >
        <h4 className="font-medium">Readiness</h4>
        <p>
          Readiness has not been computed for this project yet. Refresh the
          team.
        </p>
      </section>
    );
  }
  const summary = describeReadiness(readiness.state);
  return (
    <section
      aria-label="Project readiness"
      className="space-y-2 rounded border p-3"
    >
      <h4 className="font-medium">Readiness</h4>
      <p data-testid="project-readiness-state" role="status">
        {summary.label}
      </p>
      <p className="text-xs text-muted-foreground">
        Checked {new Date(readiness.checked_at * 1000).toLocaleString()}.{" "}
        {readiness.last_inspection_at
          ? `Access verified by isolated inspection ${new Date(readiness.last_inspection_at * 1000).toLocaleString()}.`
          : "Access has not been verified by an isolated inspection for the current binding and permissions."}
      </p>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt className="font-medium">Branch</dt>
        <dd>
          <bdi dir="ltr">{readiness.git.branch ?? "—"}</bdi>
        </dd>
        <dt className="font-medium">Revision</dt>
        <dd className="break-all">
          <bdi dir="ltr">{readiness.git.revision ?? "—"}</bdi>
        </dd>
        <dt className="font-medium">Uncommitted changes</dt>
        <dd>
          {readiness.git.dirty === null
            ? "Not verified (run the isolated inspection)"
            : readiness.git.dirty
              ? "Present — preserved, never reset"
              : "None reported"}
        </dd>
        <dt className="font-medium">Pending approvals</dt>
        <dd>{readiness.pending_approvals}</dd>
        <dt className="font-medium">Running tasks</dt>
        <dd>{readiness.running_tasks}</dd>
      </dl>
      <ul aria-label="Readiness checks" className="space-y-1">
        {readiness.checks.map((check) => (
          <li
            className="rounded border p-2 text-xs"
            data-status={check.status}
            key={check.id}
          >
            <span className="font-medium">{statusLabel(check.status)}:</span>{" "}
            <span>{check.message}</span>
            {check.recovery && check.status !== "ok" && (
              <p className="text-muted-foreground">
                Next step: {check.recovery}
              </p>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
