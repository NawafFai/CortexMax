import type { ApprovalRequest } from "@/shared/api/channelProjectTeam";

/** What each kind of request asks the owner to allow, as a verb phrase. */
export const APPROVAL_ASKS: Record<ApprovalRequest["category"], string> = {
  execute: "run a command",
  edit: "edit files",
  read: "read or attach files",
  fetch: "fetch from the network",
  commit: "commit or merge",
  publish: "publish, deploy or send something off this machine",
  channel: "manage channels",
  other: "do something outside its grants",
};

/**
 * How long the harness holds a refused call for the owner. Mirrors
 * `OWNER_DECISION_WAIT` in `crates/buzz-acp/src/project_gate.rs`; past it the
 * agent has been told no and moved on, and the copy must not claim otherwise.
 */
export const APPROVAL_HOLD_SECONDS = 5 * 60;
