import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { managementChannelNameFor } from "@/features/channels/lib/projectScaffold";
import type { FolderProjectRequest } from "@/features/projects/folderProjectRequest";
import {
  type FolderCheck,
  type FolderProjectPhase,
  useFolderProjectRequests,
} from "@/features/projects/useFolderProjectRequests";

type RequestedProject = FolderProjectRequest["request"];

const PHASE_LABEL: Record<FolderProjectPhase, string> = {
  idle: "Create project",
  pausing: "Pausing the team…",
  creating: "Creating…",
  resuming: "Starting the team again…",
};

/** What the owner reviews before a requested project is created. */
export function FolderProjectRequestDetails({
  request,
  folderCheck,
}: {
  request: RequestedProject;
  folderCheck: FolderCheck;
}) {
  return (
    <dl className="space-y-2 rounded-xl border border-border/60 bg-muted/25 p-3 text-sm">
      <div className="flex gap-3">
        <dt className="w-24 shrink-0 text-muted-foreground">Project</dt>
        <dd className="min-w-0 break-words text-foreground">#{request.name}</dd>
      </div>
      <div className="flex gap-3">
        <dt className="w-24 shrink-0 text-muted-foreground">Management</dt>
        <dd className="min-w-0 break-words text-foreground">
          #{managementChannelNameFor(request.name)}
        </dd>
      </div>
      <div className="flex gap-3">
        <dt className="w-24 shrink-0 text-muted-foreground">Folder</dt>
        <dd className="min-w-0 space-y-1">
          <p className="break-all font-mono text-xs text-foreground">
            {request.folder}
          </p>
          {folderCheck.status === "checking" ? (
            <p className="text-xs text-muted-foreground">
              Checking the folder…
            </p>
          ) : folderCheck.status === "ok" ? (
            <p className="text-xs text-emerald-600 dark:text-emerald-400">
              Folder checked. The team gets read and write access inside it.
            </p>
          ) : (
            <p className="text-xs text-destructive">
              Not allowed: {folderCheck.reason}
            </p>
          )}
        </dd>
      </div>
      {request.description ? (
        <div className="flex gap-3">
          <dt className="w-24 shrink-0 text-muted-foreground">Description</dt>
          <dd className="min-w-0 break-words text-foreground">
            {request.description}
          </dd>
        </div>
      ) : null}
      <div className="flex gap-3">
        <dt className="w-24 shrink-0 text-muted-foreground">Visibility</dt>
        <dd className="text-foreground">{request.visibility}</dd>
      </div>
    </dl>
  );
}

/** Global owner-review surface for projects an agent asks to start. */
export function FolderProjectRequestDialog() {
  const review = useFolderProjectRequests();
  const request = review.request?.request;
  const busy = review.phase !== "idle";

  return (
    <AlertDialog
      onOpenChange={(open) => {
        if (!open) review.dismiss();
      }}
      open={request != null}
    >
      <AlertDialogContent data-testid="folder-project-request-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>Start a project?</AlertDialogTitle>
          <AlertDialogDescription>
            {review.agentName ?? "Your agent"} asks to start a project in a
            folder on this Mac. It gets its own management room and a handover
            note, and {review.agentName ?? "the agent"} becomes an admin of both
            rooms. Running agents pause for a moment while it is created, then
            carry on where they stopped.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {request ? (
          <FolderProjectRequestDetails
            folderCheck={review.folderCheck}
            request={request}
          />
        ) : null}
        {review.error ? (
          <p className="text-sm text-destructive">{review.error}</p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>Not now</AlertDialogCancel>
          <AlertDialogAction
            data-testid="folder-project-request-approve"
            disabled={busy || review.folderCheck.status !== "ok"}
            onClick={(event) => {
              event.preventDefault();
              void review.approve();
            }}
          >
            {PHASE_LABEL[review.phase]}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
