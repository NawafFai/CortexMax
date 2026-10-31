import * as React from "react";
import { toast } from "sonner";

import {
  applyCostProfile,
  currentCostProfile,
  type CostProfile,
  type CostProfileOutcome,
} from "@/shared/api/tauri";

/**
 * One control that moves the whole team between model tiers.
 *
 * Spend is decided almost entirely by which model each agent runs on, but that
 * lived on each agent's own settings page — sixty-odd edits, at the exact
 * moment the owner is trying to slow a run down. These three buttons apply one
 * policy to every agent at once.
 *
 * Nothing is started or stopped, so this is safe to press mid-run: each agent
 * picks up its new model on its next launch. The control says so, because a
 * button that looks like it should stop a runaway team and does not would be
 * worse than no button.
 */

const PROFILES: {
  value: CostProfile;
  label: string;
  detail: string;
}[] = [
  {
    value: "economy",
    label: "Economy",
    detail: "Cheapest tier everywhere, one session per agent.",
  },
  {
    value: "balanced",
    label: "Balanced",
    detail: "Strong models for coordination and review, cheap for execution.",
  },
  {
    value: "max",
    label: "Max",
    detail: "Strongest tier available. What an unconstrained team costs.",
  },
];

/** Plain-language summary of what an apply actually did. */
function describe(outcome: CostProfileOutcome): string {
  const parts: string[] = [];
  if (outcome.retiered > 0) {
    const shown = outcome.examples.slice(0, 3);
    const named =
      shown.length === 0
        ? ""
        : ` (${shown.join(", ")}${outcome.retiered > shown.length ? ", …" : ""})`;
    parts.push(`${outcome.retiered} moved tier${named}`);
  }
  if (outcome.cappedOnly > 0) {
    parts.push(`${outcome.cappedOnly} capped — single-model runtime`);
  }
  if (outcome.unchanged > 0) parts.push(`${outcome.unchanged} already set`);
  if (outcome.skipped > 0) parts.push(`${outcome.skipped} skipped`);
  return parts.length > 0 ? parts.join(" · ") : "Nothing to change.";
}

export function CostProfileControl() {
  const [active, setActive] = React.useState<CostProfile | null>(null);
  const [busy, setBusy] = React.useState<CostProfile | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setActive(await currentCostProfile());
    } catch {
      // Not knowing the active profile must not disable applying one.
      setActive(null);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  async function apply(profile: CostProfile) {
    setBusy(profile);
    try {
      const outcome = await applyCostProfile(profile);
      setActive(profile);
      toast.success(`${profile} profile applied`, {
        description: describe(outcome),
      });
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : String(cause));
      void refresh();
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="space-y-2.5 px-4 py-3.5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <p className="text-xs font-medium">Cost profile</p>
        <p className="text-3xs text-muted-foreground">
          Applies on each agent&apos;s next launch — safe to press mid-run.
        </p>
      </div>

      <div className="grid gap-1.5 [@container(min-width:34rem)]:grid-cols-3">
        {PROFILES.map((profile) => {
          const isActive = active === profile.value;
          return (
            <button
              aria-pressed={isActive}
              className={`rounded-lg border px-3 py-2 text-left transition-colors ${
                isActive
                  ? "border-foreground/30 bg-muted"
                  : "border-border/70 hover:bg-muted/60"
              } ${busy != null ? "opacity-60" : ""}`}
              disabled={busy != null}
              key={profile.value}
              onClick={() => void apply(profile.value)}
              type="button"
            >
              <span className="flex items-baseline justify-between gap-2">
                <span className="text-xs font-medium">{profile.label}</span>
                {isActive ? (
                  <span className="shrink-0 text-3xs text-muted-foreground">
                    active
                  </span>
                ) : null}
              </span>
              <span className="mt-0.5 block text-3xs leading-relaxed text-muted-foreground">
                {profile.detail}
              </span>
            </button>
          );
        })}
      </div>

      {active == null ? (
        <p className="text-3xs text-muted-foreground">
          Your agents are on a mix of tiers, so no profile is shown as active.
          Pick one to bring the whole team onto it.
        </p>
      ) : null}
    </div>
  );
}
