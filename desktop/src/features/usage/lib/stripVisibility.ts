import type { LimitGroup } from "@/shared/api/tauri";

/**
 * Which runtimes belong in the sidebar strip.
 *
 * The strip answers one question — how close am I to a ceiling that will stop
 * the team — so it carries only runtimes that actually report a window. A
 * provider with nothing to report still has a reason worth reading (Kimi keeps
 * its figures inside its own interactive CLI; OpenCode publishes no plan limit
 * at all, only spend history), but a paragraph of explanation in a narrow
 * strip costs the space the real numbers need and answers nothing at a glance.
 *
 * Those reasons are not discarded: the Usage dashboard shows every group,
 * unreadable ones included, because that is where an owner goes to find out
 * why a number is missing rather than to read the number.
 */
export function stripGroups(groups: readonly LimitGroup[]): LimitGroup[] {
  return groups.filter((group) => group.windows.length > 0);
}
