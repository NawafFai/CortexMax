import * as React from "react";

import { agentRollCall, type LeaveEntry } from "@/shared/api/tauri";
import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";

/**
 * Who on the team is out of runway, until when, and who is covering.
 *
 * A spent limit does not stop an agent politely — it fails mid-turn, retries,
 * and fails again, so an agent with nothing left looks exactly like one hard
 * at work. Two of this team's managers burned 38 and 26 retries that way while
 * appearing busy. Naming the state is the whole point: an agent that cannot
 * work should say so, with the provider's own reset as the return date.
 *
 * A cover is shown with its reasoning attached whenever it was suggested
 * rather than chosen. The app knows what an agent runs, not what it is for, so
 * picking a stand-in from a name would be a guess; labelling the guess keeps
 * it a suggestion instead of letting it read as a decision.
 */
export function RollCallCard() {
  const [away, setAway] = React.useState<LeaveEntry[] | null>(null);

  const load = React.useCallback(async () => {
    try {
      setAway(await agentRollCall());
    } catch {
      // No readable limits is the ordinary state before any runtime has been
      // used; no card is the honest rendering, not an error banner.
      setAway(null);
    }
  }, []);

  React.useEffect(() => {
    void load();
    // The reset times move on their own, so the badge would otherwise go stale
    // while the page sits open. A minute is finer than any window it reports.
    const timer = setInterval(() => void load(), 60_000);
    return () => clearInterval(timer);
  }, [load]);

  // Nobody away is the good state, and an empty card announcing it is noise.
  if (away == null || away.length === 0) return null;

  return (
    <SettingsOptionGroup title="On leave">
      <div className="space-y-2.5 px-4 py-3.5">
        <p className="text-xs text-muted-foreground">
          {away.length} agent{away.length === 1 ? " is" : "s are"} out of
          runway. This comes from each provider&apos;s own window, not from
          watching them fail.
        </p>
        <ul className="-mx-4 divide-y divide-border/55 border-t border-border/55">
          {away.map((entry) => (
            <li className="px-4 py-2.5 text-xs" key={entry.pubkey}>
              <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5">
                <span className="font-medium">{entry.name}</span>
                <span className="shrink-0 text-amber-600 tabular-nums dark:text-amber-400">
                  {entry.label}
                </span>
              </div>
              <p className="mt-1 text-3xs leading-relaxed text-muted-foreground">
                {entry.cover
                  ? entry.coverNote
                    ? `Suggested cover: ${entry.cover} — ${entry.coverNote}.`
                    : `Covered by ${entry.cover}.`
                  : `No cover — ${entry.coverNote ?? "none available"}.`}
              </p>
            </li>
          ))}
        </ul>
      </div>
    </SettingsOptionGroup>
  );
}
