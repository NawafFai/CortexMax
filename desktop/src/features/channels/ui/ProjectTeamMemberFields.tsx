import type {
  ProjectCapability,
  ProjectEnforcement,
  ProjectRole,
  ProjectTeamMember,
} from "@/shared/api/channelProjectTeam";
import { Button } from "@/shared/ui/button";

const capabilityLabels: Record<ProjectCapability, string> = {
  read: "Read project files",
  edit: "Edit project files",
  run_checks: "Run project checks (approved commands only)",
  manage_team: "Manage this project team",
  assign_tasks: "Plan and assign project tasks",
  review: "Review deliveries (independent reviewer)",
  create_private_channels: "Create or invite to authorized private channels",
  commit_merge: "Commit or merge locally (never push)",
};

const roleLabels: Record<ProjectRole, string> = {
  coordinator: "Coordinator",
  manager: "Manager",
  writer: "Writer",
  tester: "Tester",
  reviewer: "Independent reviewer",
  member: "Team member",
};

export function enforcementSummary(
  enforcement: ProjectEnforcement | undefined,
) {
  if (!enforcement) return "Enforcement not reported";
  switch (enforcement.kind) {
    case "native_sandbox":
      return "Native sandboxed project tools";
    case "gate": {
      const p = enforcement.profile;
      return `Harness gate — writes ${p.write_confinement.replace("_", " ")}, reads ${p.read_confinement.replace("_", " ")}, commands ${p.command_confinement.replace("_", " ")}, network ${p.network_blocked ? "blocked" : "open"}`;
    }
    case "unverified":
      return `No verified enforcement: ${enforcement.reason}`;
  }
}

export function ProjectTeamMemberFields({
  member,
  runtime,
  enforcement,
  roles,
  capabilities,
  onChange,
  onRelease,
}: {
  member: ProjectTeamMember;
  runtime: string | null;
  enforcement: ProjectEnforcement | undefined;
  roles: ProjectRole[];
  capabilities: ProjectCapability[];
  onChange: (member: ProjectTeamMember) => void;
  onRelease: () => void;
}) {
  return (
    <fieldset className="space-y-2 rounded border p-3">
      <legend className="px-1 font-medium">{member.display_name}</legend>
      <p className="break-all text-xs text-muted-foreground">
        <bdi dir="ltr">{member.pubkey}</bdi>
      </p>
      <p className="text-xs text-muted-foreground">
        Runtime: <bdi dir="ltr">{runtime ?? "Not reported"}</bdi> ·{" "}
        {enforcementSummary(enforcement)}
      </p>
      <label className="block space-y-1">
        <span>Project role</span>
        <select
          className="block w-full rounded border bg-background p-2"
          disabled={member.role === "coordinator"}
          onChange={(event) =>
            onChange({
              ...member,
              role: event.target.value as ProjectRole,
            })
          }
          value={member.role}
        >
          {roles
            .filter((role) =>
              member.role === "coordinator"
                ? role === "coordinator"
                : role !== "coordinator",
            )
            .map((role) => (
              <option key={role} value={role}>
                {roleLabels[role] ?? role}
              </option>
            ))}
        </select>
      </label>
      <div className="space-y-1">
        {capabilities.map((value) => (
          <label className="flex items-center gap-2" key={value}>
            <input
              checked={member.capabilities.includes(value)}
              onChange={(event) =>
                onChange({
                  ...member,
                  capabilities: event.target.checked
                    ? [...member.capabilities, value]
                    : member.capabilities.filter((item) => item !== value),
                })
              }
              type="checkbox"
            />
            {capabilityLabels[value] ?? value}
          </label>
        ))}
      </div>
      <p className="text-xs text-muted-foreground">
        Publishing, deploying, purchasing and destructive actions are never
        grantable; each request is decided in the Approvals tab.
      </p>
      <Button onClick={onRelease} type="button" variant="outline">
        Release from project
      </Button>
    </fieldset>
  );
}
