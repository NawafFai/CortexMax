/**
 * An agent asking its owner to start a project: a channel bound to a folder on
 * the owner's machine, made with its management room and handover note.
 *
 * Nothing is created from the request itself. It opens a confirmation the
 * owner answers, which is what keeps a project — and the file access an agent
 * team gets inside it — the owner's decision. `buzz channels draft-project`
 * sends it.
 */
export const FOLDER_PROJECT_REQUEST = "folder_project_request" as const;

export type FolderProjectRequest = {
  type: typeof FOLDER_PROJECT_REQUEST;
  action: "create";
  requestId: string;
  request: {
    /** The channel the request came from, where the result is reported. */
    channelId: string;
    name: string;
    /** Absolute folder path, canonical as the CLI resolved it. */
    folder: string;
    description?: string;
    visibility: "open" | "private";
  };
};

const UUID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

function isTextWithin(value: unknown, max: number): value is string {
  return (
    typeof value === "string" && value.trim().length > 0 && value.length <= max
  );
}

/** A control character anywhere in a path is a forged one, not a typo. */
function hasControlCharacter(value: string): boolean {
  for (const character of value) {
    const code = character.codePointAt(0) ?? 0;
    if (code < 0x20 || code === 0x7f) return true;
  }
  return false;
}

/** Parse the narrow, no-secret contract; anything unexpected is refused. */
export function parseFolderProjectRequest(
  value: unknown,
): FolderProjectRequest | null {
  if (typeof value !== "object" || value === null) return null;
  const payload = value as Record<string, unknown>;
  if (
    payload.type !== FOLDER_PROJECT_REQUEST ||
    payload.action !== "create" ||
    !isTextWithin(payload.requestId, 128) ||
    typeof payload.request !== "object" ||
    payload.request === null
  ) {
    return null;
  }
  const request = payload.request as Record<string, unknown>;
  const allowed = ["channelId", "name", "folder", "description", "visibility"];
  if (
    Object.keys(request).some((key) => !allowed.includes(key)) ||
    !isTextWithin(request.channelId, 128) ||
    !UUID.test(request.channelId) ||
    !isTextWithin(request.name, 120) ||
    !isTextWithin(request.folder, 4_096) ||
    !request.folder.startsWith("/") ||
    hasControlCharacter(request.folder) ||
    (request.visibility !== "open" && request.visibility !== "private") ||
    (request.description !== undefined &&
      !isTextWithin(request.description, 2_048))
  ) {
    return null;
  }
  return {
    type: FOLDER_PROJECT_REQUEST,
    action: "create",
    requestId: payload.requestId,
    request: {
      channelId: request.channelId,
      name: request.name.trim(),
      folder: request.folder,
      visibility: request.visibility,
      ...(request.description
        ? { description: request.description.trim() }
        : {}),
    },
  };
}

/** The line the owner's app posts back where the agent asked. */
export function folderProjectCreatedNote(input: {
  agentName: string;
  workName: string;
  workId: string;
  managementName: string | null;
  managementId: string | null;
  folder: string;
}): string {
  const management =
    input.managementName && input.managementId
      ? ` and #${input.managementName} (${input.managementId})`
      : "";
  return (
    `@${input.agentName} The project is created: #${input.workName} (${input.workId})${management}, ` +
    `bound to ${input.folder}. You are an admin of ${management ? "both" : "it"}; the handover note is in the folder.`
  );
}
