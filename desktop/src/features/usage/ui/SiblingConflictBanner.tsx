import * as React from "react";
import { TriangleAlert } from "lucide-react";

import { siblingConflicts } from "@/shared/api/tauri";
import type { ConflictReport } from "@/shared/api/tauri";

/**
 * Warn when another Buzz-family app holds the same agent identities.
 *
 * Cortex Max is a fork, so a machine that ran upstream Buzz first has both
 * apps holding the same keypairs. Nothing stops both from running at once, and
 * when they do, two processes sign as one agent on one relay: turns interleave,
 * one app's reply answers the other's prompt, and the duplicated identity burns
 * two subscriptions on a single conversation.
 *
 * The banner distinguishes the two states, because they call for different
 * things. Shared-but-idle is something to know before opening the other app.
 * Shared-and-running is happening now and wants action now.
 */
export function SiblingConflictBanner() {
  const [reports, setReports] = React.useState<ConflictReport[]>([]);

  React.useEffect(() => {
    void (async () => {
      try {
        setReports(await siblingConflicts());
      } catch {
        // A failed check must not block the page it sits on; the absence of a
        // banner is the same as the absence of a conflict from here.
        setReports([]);
      }
    })();
  }, []);

  const conflicts = reports.filter((report) => report.severity !== "none");
  if (conflicts.length === 0) return null;

  return (
    <>
      {conflicts.map((report) => {
        const live = report.severity === "critical";
        return (
          <div
            className={`flex gap-2.5 px-4 py-3 ${
              live ? "bg-red-500/10" : "bg-amber-500/10"
            }`}
            key={report.bundleId}
          >
            <TriangleAlert
              className={`mt-0.5 shrink-0 ${
                live
                  ? "text-red-600 dark:text-red-400"
                  : "text-amber-600 dark:text-amber-400"
              }`}
              size={14}
            />
            <div className="min-w-0 space-y-1">
              <p className="text-xs font-medium">
                {live
                  ? `Another app is running ${report.runningSharedAgents} of your agents right now`
                  : `Another app holds the same ${report.sharedAgents} agent identities`}
              </p>
              <p className="text-3xs leading-relaxed text-muted-foreground">
                {report.bundleId} shares {report.sharedAgents} agent
                {report.sharedAgents === 1 ? "" : "s"} with Cortex Max
                {report.sampleNames.length > 0
                  ? ` (${report.sampleNames.join(", ")}…)`
                  : ""}
                .{" "}
                {live
                  ? "Both apps are signing as the same agents on the same relay, so their turns interleave and each conversation is paid for twice. Quit the other app, or stop its agents."
                  : "Running both at once would make them sign as the same agents on the same relay, so keep one closed."}
              </p>
            </div>
          </div>
        );
      })}
    </>
  );
}
