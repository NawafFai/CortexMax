import type { CreateChannelFormState } from "../lib/useCreateChannelForm";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

export function ChannelProjectCreationFields({
  form,
}: {
  form: CreateChannelFormState;
}) {
  const project = form.project;
  const disabled = form.isCreating || project.busy;
  return (
    <fieldset className="space-y-3 rounded-xl border p-3" disabled={disabled}>
      <legend className="px-1 text-sm font-medium">Local project</legend>
      <label className="block space-y-1 text-sm">
        <span>Channel workspace</span>
        <select
          className="block w-full rounded border bg-background p-2"
          value={project.mode}
          onChange={(event) =>
            project.setMode(event.target.value as typeof project.mode)
          }
        >
          <option value="conversation">Conversation only</option>
          <option value="existing">Attach existing local project</option>
          <option value="new">Create new local project</option>
        </select>
      </label>
      {project.pending.data && (
        <div className="space-y-2 text-sm">
          <p>
            A saved project channel creation needs reconciliation. Resume it
            instead of creating a duplicate.
          </p>
          <Button
            type="button"
            variant="outline"
            onClick={form.resumeProjectCreation}
          >
            Resume saved creation
          </Button>
          <label className="flex items-start gap-2">
            <input
              type="checkbox"
              checked={project.abandonConfirmed}
              onChange={(event) =>
                project.setAbandonConfirmed(event.target.checked)
              }
            />
            I inspected the relay and verified it is safe to abandon this local
            draft. Any remote channel and local files will remain.
          </label>
          <Button
            type="button"
            variant="outline"
            disabled={
              !project.abandonConfirmed ||
              project.pending.isFetching ||
              project.pending.isError
            }
            onClick={form.abandonProjectCreation}
          >
            Abandon local draft
          </Button>
        </div>
      )}
      {project.mode !== "conversation" && (
        <>
          <p className="text-sm">
            Paths stay on this device. After creating this channel, open Local
            project to choose its coordinator and team, grant permissions and
            check execution readiness. Templates and temporary channels are not
            supported with local projects yet.
          </p>
          <Button
            type="button"
            variant="outline"
            onClick={() => void project.pick()}
          >
            {project.mode === "new"
              ? "Choose parent folder"
              : "Choose project folder"}
          </Button>
          {project.parent && (
            <p className="break-all text-sm">
              <bdi dir="ltr">{project.parent}</bdi>
            </p>
          )}
          {project.mode === "new" && (
            <>
              <label
                className="block space-y-1 text-sm"
                htmlFor="new-channel-project-folder-name"
              >
                <span>New project folder name</span>
                <Input
                  id="new-channel-project-folder-name"
                  value={project.folderName}
                  disabled={!!project.root}
                  onChange={(event) =>
                    project.setFolderName(event.target.value)
                  }
                />
              </label>
              <Button
                type="button"
                variant="outline"
                disabled={
                  !project.parent ||
                  !project.folderName.trim() ||
                  !!project.root
                }
                onClick={() => void project.createFolder()}
              >
                Create empty folder
              </Button>
              <p className="text-sm">
                This creates a folder now, not a Git commit. Existing files are
                never replaced. Closing this form does not delete it.
              </p>
            </>
          )}
          {project.root && (
            <p className="break-all text-sm">
              <bdi dir="ltr">{project.root}</bdi>
            </p>
          )}
          {project.pending.error && (
            <p role="alert" className="text-sm">
              {String(project.pending.error)}
            </p>
          )}
          <Button
            type="button"
            variant="outline"
            disabled={project.pending.isFetching}
            onClick={() => void project.pending.refetch()}
          >
            Refresh saved creation
          </Button>
          <p className="text-sm">
            Choose the coordinator and project-specific permissions from the
            channel header after creation. No agents are created or started
            automatically.
          </p>
        </>
      )}
      {project.error && (
        <p role="alert" className="text-sm">
          {project.error}
        </p>
      )}
    </fieldset>
  );
}
