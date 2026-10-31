import { runtimeFamily } from "@/features/agents/lib/runtimeFamily";
import { cn } from "@/shared/lib/cn";

/**
 * Which model family runs an agent, on its card.
 *
 * A thirty-agent team runs on four model families with four separate limits,
 * and which one an agent spends decides whether it can work once a limit is
 * reached. The card face said nothing about it; the answer took opening the
 * agent. Families are named the way the owner names them: Codex is "ChatGPT".
 *
 * The model decides, not the harness: a Codex harness pointed at a Kimi model
 * spends Kimi's quota, so it is a Kimi agent. The harness only answers when no
 * model is set.
 */
type RuntimeBrand = { label: string; title: string; className: string };

const BRANDS: Record<string, RuntimeBrand> = {
  claude: {
    label: "Claude",
    title: "Claude (Anthropic)",
    className:
      "bg-orange-500/15 text-orange-700 ring-orange-500/30 dark:text-orange-300",
  },
  codex: {
    label: "ChatGPT",
    title: "ChatGPT (OpenAI Codex)",
    className:
      "bg-emerald-500/15 text-emerald-700 ring-emerald-500/30 dark:text-emerald-300",
  },
  kimi: {
    label: "Kimi",
    title: "Kimi (Moonshot)",
    className: "bg-sky-500/15 text-sky-700 ring-sky-500/30 dark:text-sky-300",
  },
  opencode: {
    label: "OpenCode",
    title: "OpenCode",
    className:
      "bg-zinc-500/15 text-zinc-700 ring-zinc-500/30 dark:text-zinc-300",
  },
};

export function RuntimeBadge({
  runtime,
  command,
  model,
}: {
  runtime: string | null | undefined;
  command?: string | null;
  model?: string | null;
}) {
  const family = runtimeFamily(runtime, command, model);
  if (!family) return null;
  const brand = BRANDS[family] ?? {
    label: family,
    title: family,
    className: "bg-muted text-muted-foreground ring-border",
  };
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2 py-0.5 text-2xs font-semibold leading-4 ring-1 ring-inset backdrop-blur-sm",
        brand.className,
      )}
      data-testid="agent-runtime-badge"
      title={brand.title}
    >
      {brand.label}
    </span>
  );
}
