import * as React from "react";
import { toast } from "sonner";

import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  TEAM_TEMPLATES,
  planTemplate,
  type TeamTemplate,
} from "@/features/agents/lib/teamTemplates";
import { runtimeTeamLabel } from "@/features/agents/lib/teamRoster";
import { createPersona, listPersonas } from "@/shared/api/tauriPersonas";

/**
 * Ready-made teams, so a new project does not start from an empty roster.
 *
 * Applying a template creates **definitions**, never live agents. A definition
 * has no keypair and no relay identity — it is a description the owner deploys
 * deliberately afterwards. That is the whole safety of this card: it cannot put
 * a second agent on the relay under an identity that already exists, and it
 * cannot touch the running team.
 *
 * A role whose name is already taken is skipped rather than overwritten. The
 * existing agent may be the owner's own, with its own prompt and history, and
 * replacing it silently would be the most destructive thing this could do.
 */

/** Runtimes a template can be created on. */
const RUNTIMES = ["claude", "codex", "kimi"] as const;

/** The model tier a role gets, matching the Balanced cost profile. */
function modelFor(runtime: string, deep: boolean): string | undefined {
  if (runtime === "claude") {
    return deep ? "claude-sonnet-5" : "claude-haiku-4-5-20251001";
  }
  if (runtime === "codex") return "gpt-5.6-terra";
  if (runtime === "kimi") {
    // Kimi's own catalogue, read after `kimi login`. The split is the
    // provider's own naming: "highspeed" is the fast tier.
    return deep
      ? "kimi-code/kimi-for-coding"
      : "kimi-code/kimi-for-coding-highspeed";
  }
  return undefined;
}

function TemplateRow({
  template,
  runtime,
  existingNames,
  onApplied,
}: {
  template: TeamTemplate;
  runtime: string;
  existingNames: string[];
  onApplied: () => void;
}) {
  const [busy, setBusy] = React.useState(false);
  const plan = React.useMemo(
    () => planTemplate(template, existingNames),
    [template, existingNames],
  );

  async function apply() {
    setBusy(true);
    const created: string[] = [];
    const failed: string[] = [];
    try {
      for (const role of plan.create) {
        try {
          await createPersona({
            displayName: role.name,
            avatarUrl: role.avatarUrl,
            description: role.description,
            systemPrompt: role.systemPrompt,
            runtime,
            model: modelFor(runtime, role.deep),
          });
          created.push(role.name);
        } catch (cause) {
          // One refused role must not abandon the rest, and must not be
          // reported as created.
          failed.push(
            `${role.name}: ${cause instanceof Error ? cause.message : String(cause)}`,
          );
        }
      }

      if (created.length > 0) {
        toast.success(
          `${created.length} agent definition${created.length === 1 ? "" : "s"} created`,
          {
            description: `${created.join(", ")}. Deploy them from Agents when you are ready — nothing is running yet.`,
          },
        );
      }
      if (failed.length > 0) toast.error(failed[0]);
      if (created.length === 0 && failed.length === 0) {
        toast.info("Every role in this template already exists.");
      }
      onApplied();
    } finally {
      setBusy(false);
    }
  }

  return (
    <li className="space-y-2 px-4 py-3">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <div className="min-w-0">
          <p className="text-xs font-medium">{template.name}</p>
          <p className="text-3xs leading-relaxed text-muted-foreground">
            {template.summary}
          </p>
        </div>
        <button
          className="shrink-0 rounded border px-2.5 py-1 text-xs transition-colors hover:bg-muted/60 disabled:opacity-60"
          disabled={busy || plan.create.length === 0}
          onClick={() => void apply()}
          type="button"
        >
          {plan.create.length === 0
            ? "Already added"
            : `Add ${plan.create.length}`}
        </button>
      </div>

      <ul className="flex flex-wrap gap-1.5">
        {template.roles.map((role) => {
          const exists = plan.skip.includes(role);
          return (
            <li
              className={`rounded border px-1.5 py-0.5 text-3xs ${
                exists
                  ? "border-border/50 text-muted-foreground line-through"
                  : "border-border/70"
              }`}
              key={role.name}
              title={
                exists
                  ? `${role.name} already exists and will not be touched`
                  : role.description
              }
            >
              {role.name}
              {role.deep ? (
                <span className="ml-1 text-muted-foreground">·lead</span>
              ) : null}
            </li>
          );
        })}
      </ul>
    </li>
  );
}

export function TeamTemplatesCard() {
  const agentsQuery = useManagedAgentsQuery();
  const [runtime, setRuntime] = React.useState<string>("claude");
  const [definitionNames, setDefinitionNames] = React.useState<string[]>([]);

  const loadDefinitions = React.useCallback(async () => {
    try {
      setDefinitionNames(
        (await listPersonas()).map((persona) => persona.displayName),
      );
    } catch {
      // Without the definition list the card still works; it simply cannot
      // grey out roles that already exist, and creating one would be refused
      // by the store rather than overwrite anything.
      setDefinitionNames([]);
    }
  }, []);

  React.useEffect(() => {
    void loadDefinitions();
  }, [loadDefinitions]);

  // Both halves count as "taken": a live agent and a definition can share a
  // name, and either makes the role already present as far as the owner cares.
  const existingNames = React.useMemo(
    () => [
      ...definitionNames,
      ...(agentsQuery.data ?? []).map((agent) => agent.name),
    ],
    [definitionNames, agentsQuery.data],
  );

  return (
    <SettingsOptionGroup
      description="Ready-made teams. Adding one creates agent definitions you then deploy — nothing starts running by itself."
      title="Team templates"
    >
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-4 py-3">
        <span className="text-xs font-medium">Create on</span>
        <div className="flex items-center gap-1.5">
          {RUNTIMES.map((id) => (
            <button
              aria-pressed={runtime === id}
              className={`rounded border px-2.5 py-1 text-xs transition-colors ${
                runtime === id
                  ? "border-foreground/30 bg-muted font-medium"
                  : "border-transparent text-muted-foreground hover:bg-muted/60"
              }`}
              key={id}
              onClick={() => setRuntime(id)}
              type="button"
            >
              {runtimeTeamLabel(id)}
            </button>
          ))}
        </div>
      </div>

      <ul className="divide-y divide-border/55">
        {TEAM_TEMPLATES.map((template) => (
          <TemplateRow
            existingNames={existingNames}
            key={template.id}
            onApplied={() => void loadDefinitions()}
            runtime={runtime}
            template={template}
          />
        ))}
      </ul>
    </SettingsOptionGroup>
  );
}
