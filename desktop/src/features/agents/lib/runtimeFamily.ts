/**
 * The model family an agent spends: Claude, Codex (which the owner calls
 * ChatGPT), Kimi or OpenCode.
 *
 * The model decides, not the harness: a Codex harness pointed at a Kimi model
 * spends Kimi's quota, so it is a Kimi agent. The harness only answers when no
 * model is set, and the launch command when no harness id is.
 */

/** The families there are. */
const FAMILIES = new Set(["claude", "codex", "kimi", "opencode"]);

/** Harness ids and commands that belong to each family. */
const ALIASES: Record<string, string> = {
  "claude-agent-acp": "claude",
  "claude-code-acp": "claude",
  "codex-acp": "codex",
  "kimi-code": "kimi",
};

/** The family a model id belongs to, when it names one. */
export function modelFamily(model: string | null | undefined): string | null {
  const id = model?.trim().toLowerCase();
  if (!id) return null;
  if (id.startsWith("opencode/")) return "opencode";
  if (/^(kimi|moonshot)|(^|\/)k\d/.test(id)) return "kimi";
  if (/^(claude|anthropic|opus|sonnet|haiku|fable)/.test(id)) return "claude";
  if (/^(gpt|o\d|codex|chatgpt|openai)/.test(id)) return "codex";
  return null;
}

/**
 * The family for an agent: its model's when the model names one, else its
 * runtime id's, else the command that launches it.
 */
export function runtimeFamily(
  runtime: string | null | undefined,
  command?: string | null,
  model?: string | null,
): string | null {
  const fromModel = modelFamily(model);
  if (fromModel) return fromModel;
  const id = runtime?.trim().toLowerCase();
  if (id) return ALIASES[id] ?? id;
  const executable = command?.trim().split(/\s+/)[0]?.split("/").pop();
  if (!executable) return null;
  const name = executable.toLowerCase();
  return ALIASES[name] ?? (FAMILIES.has(name) ? name : null);
}
