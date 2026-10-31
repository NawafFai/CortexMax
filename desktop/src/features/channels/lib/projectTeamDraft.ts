import type {
  ProjectTeamDraft,
  ProjectTeamMember,
  ProjectTeamStatus,
} from "@/shared/api/channelProjectTeam";

export const MAX_APPROVED_CHECKS = 64;
export const MAX_CHECK_COMMAND_BYTES = 512;

export function projectTeamDraft(status: ProjectTeamStatus): ProjectTeamDraft {
  return {
    generation: status.generation,
    binding_generation: status.binding_generation,
    coordinator_pubkey: status.coordinator_pubkey,
    paused: status.paused,
    approved_checks: [...(status.approved_checks ?? [])],
    members: status.members.map((member) => ({
      ...member,
      capabilities: [...member.capabilities],
    })),
  };
}

export function selectProjectCoordinator(
  draft: ProjectTeamDraft,
  status: ProjectTeamStatus,
  pubkey: string | null,
): ProjectTeamDraft {
  const members: ProjectTeamMember[] = draft.members.map((member) => ({
    ...member,
    role: member.role === "coordinator" ? "member" : member.role,
  }));
  if (pubkey !== null && !members.some((member) => member.pubkey === pubkey)) {
    const agent = status.available_agents.find(
      (agent) => agent.pubkey === pubkey,
    );
    if (!agent) throw new Error("Choose an existing agent for this project.");
    members.push({
      pubkey,
      display_name: agent.display_name,
      role: "member",
      capabilities: ["read"],
    });
  }
  return {
    ...draft,
    coordinator_pubkey: pubkey,
    members: members.map((member) => ({
      ...member,
      role: member.pubkey === pubkey ? "coordinator" : member.role,
    })),
  };
}

export function releaseProjectMember(
  draft: ProjectTeamDraft,
  pubkey: string,
): ProjectTeamDraft {
  return {
    ...draft,
    coordinator_pubkey:
      draft.coordinator_pubkey === pubkey ? null : draft.coordinator_pubkey,
    members: draft.members.filter((member) => member.pubkey !== pubkey),
  };
}

/** Validate one approved check line the way the native store does. */
export function approvedCheckError(command: string): string | null {
  if (command.trim() !== command || command.length === 0)
    return "Approved checks must be trimmed and non-empty.";
  if (new TextEncoder().encode(command).length > MAX_CHECK_COMMAND_BYTES)
    return "An approved check must be at most 512 bytes.";
  if (/[\p{Cc}]/u.test(command))
    return "An approved check must be a single printable line.";
  return null;
}

export function addApprovedCheck(
  draft: ProjectTeamDraft,
  command: string,
): ProjectTeamDraft {
  const trimmed = command.trim();
  const error = approvedCheckError(trimmed);
  if (error) throw new Error(error);
  if (draft.approved_checks.includes(trimmed)) return draft;
  if (draft.approved_checks.length >= MAX_APPROVED_CHECKS)
    throw new Error("Too many approved checks.");
  return { ...draft, approved_checks: [...draft.approved_checks, trimmed] };
}

export function removeApprovedCheck(
  draft: ProjectTeamDraft,
  command: string,
): ProjectTeamDraft {
  return {
    ...draft,
    approved_checks: draft.approved_checks.filter((item) => item !== command),
  };
}

export function projectTeamSaveError(
  draft: ProjectTeamDraft,
  status: ProjectTeamStatus,
): string | null {
  if (draft.binding_generation !== status.binding_generation)
    return "Project folder changed. Reload and review its permissions before saving.";
  if (draft.generation !== status.generation)
    return "Project permissions changed. Reload before saving this draft.";
  const identities = new Set<string>();
  for (const member of draft.members) {
    if (identities.has(member.pubkey)) return "An agent can only join once.";
    identities.add(member.pubkey);
    if (
      !status.available_agents.some((agent) => agent.pubkey === member.pubkey)
    )
      return "A selected agent is no longer available. Reload the team.";
    if (
      (member.role === "coordinator") !==
      (draft.coordinator_pubkey === member.pubkey)
    )
      return "Choose exactly one project coordinator, or none.";
  }
  if (draft.coordinator_pubkey && !identities.has(draft.coordinator_pubkey))
    return "The coordinator must be a project team member.";
  const seen = new Set<string>();
  for (const command of draft.approved_checks) {
    const error = approvedCheckError(command);
    if (error) return error;
    if (seen.has(command)) return "Approved checks must be distinct.";
    seen.add(command);
  }
  if (draft.approved_checks.length > MAX_APPROVED_CHECKS)
    return "Too many approved checks.";
  return null;
}
