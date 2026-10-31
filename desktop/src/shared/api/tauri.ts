import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import {
  fromRawInstallRuntimeResult,
  type RawInstallRuntimeResult,
} from "@/shared/api/installTypes";
import type {
  AddChannelMembersInput,
  AddChannelMembersResult,
  BackendProviderCandidate,
  BackendProviderProbeResult,
  CanvasResponse,
  GetHomeFeedInput,
  HomeFeedResponse,
  ManagedAgent,
  ManagedAgentBackend,
  RelayAgent,
  RelayMember,
  RelayMemberRole,
  PresenceLookup,
  PresenceStatus,
  RelayEvent,
  SearchMessagesInput,
  SearchMessagesResponse,
  SetCanvasInput,
  SetCanvasResult,
  ThreadCursor,
  ThreadRepliesResponse,
  CreateManagedAgentInput,
  AgentModelsResponse,
  UpdateManagedAgentInput,
  AcpAvailabilityStatus,
  AcpRuntimeCatalogEntry,
  AuthStatus,
  CommandAvailability,
  InstallRuntimeResult,
  GitBashPrerequisite,
  RuntimeConfigSurface,
} from "@/shared/api/types";

export * from "@/shared/api/tauriChannels";
export { sendChannelMessage } from "@/shared/api/tauriMessages";
export { getEventById, getEventsByIds } from "@/shared/api/tauriEvents";

type RawPresenceLookup = Record<string, PresenceStatus>;

type RawAddChannelMembersResult = {
  added: string[];
  errors: Array<{
    pubkey: string;
    error: string;
  }>;
};

type RawFeedItem = {
  id: string;
  kind: number;
  pubkey: string;
  content: string;
  created_at: number;
  channel_id: string | null;
  channel_name: string;
  channel_type: string | null;
  tags: string[][];
  category: HomeFeedResponse["feed"]["mentions"][number]["category"];
};

type RawHomeFeedResponse = {
  feed: {
    mentions: RawFeedItem[];
    needs_action: RawFeedItem[];
    activity: RawFeedItem[];
    agent_activity: RawFeedItem[];
  };
  meta: {
    since: number;
    total: number;
    generated_at: number;
  };
};

type RawSearchHit = {
  event_id: string;
  content: string;
  kind: number;
  pubkey: string;
  channel_id: string | null;
  channel_name: string | null;
  created_at: number;
  score: number;
};

type RawSearchResponse = {
  hits: RawSearchHit[];
  found: number;
};

type RawRelayAgent = {
  pubkey: string;
  owner_pubkey?: string | null;
  name: string;
  agent_type: string;
  channels: string[];
  channel_ids: string[];
  capabilities: string[];
  status: RelayAgent["status"];
  respond_to?: RelayAgent["respondTo"];
  respond_to_allowlist?: string[];
};
import type { RestartDiffEntry as RawRestartDiffEntry } from "./restartDiff";
export type RawManagedAgent = {
  pubkey: string;
  name: string;
  persona_id: string | null;
  // Optional: pre-feature fixtures may omit it. The record's harness/runtime id.
  runtime?: string | null;
  team_id?: string | null;
  /** The agent this one reports to. Absent or null = reports to the owner. */
  manager_pubkey?: string | null;
  relay_url: string;
  acp_command: string;
  agent_command: string;
  agent_command_override?: string | null;
  agent_args: string[];
  mcp_command: string;
  turn_timeout_seconds: number;
  idle_timeout_seconds: number | null;
  max_turn_duration_seconds: number | null;
  parallelism: number;
  session_policy?: ManagedAgent["sessionPolicy"];
  system_prompt: string | null;
  avatar_url?: string | null;
  model: string | null;
  model_source?: ManagedAgent["modelSource"];
  provider: string | null;
  persona_out_of_date: boolean;
  persona_orphaned: boolean;
  needs_restart: boolean;
  restart_diff?: RawRestartDiffEntry[];
  env_vars?: Record<string, string>;
  status: ManagedAgent["status"];
  pid: number | null;
  created_at: string;
  updated_at: string;
  last_started_at: string | null;
  last_stopped_at: string | null;
  last_exit_code: number | null;
  last_error: string | null;
  last_error_code: number | null;
  log_path: string;
  start_on_app_launch: boolean;
  auto_restart_on_config_change?: boolean;
  backend: ManagedAgentBackend;
  backend_agent_id: string | null;
  // Pre-feature fixtures may omit these; mapped to "owner-only"/[] in fromRawManagedAgent.
  respond_to?: ManagedAgent["respondTo"];
  respond_to_allowlist?: string[];
};

type RawCreateManagedAgentResponse = {
  agent: RawManagedAgent;
  private_key_nsec: string;
  profile_sync_error: string | null;
  spawn_error: string | null;
};

type RawManagedAgentLog = {
  content: string;
  log_path: string;
};

export type RawAcpRuntimeCatalogEntry = {
  id: string;
  label: string;
  avatar_url: string;
  availability: AcpAvailabilityStatus;
  command: string | null;
  binary_path: string | null;
  default_args: string[];
  mcp_command: string | null;
  model_env_var?: string | null;
  provider_env_var?: string | null;
  thinking_env_var?: string | null;
  max_tokens_env_var?: string | null;
  context_limit_env_var?: string | null;
  max_rounds_env_var?: string | null;
  install_hint: string;
  install_instructions_url: string;
  can_auto_install: boolean;
  requires_external_cli?: boolean;
  underlying_cli_path: string | null;
  node_required: boolean;
  auth_status: AuthStatus;
  login_hint?: string;
  source: "builtin" | "preset" | "custom";
  /** Definition-level env vars for `source: custom` entries; absent for builtin/preset. */
  definition_env?: Record<string, string>;
  max_parallelism?: number;
  effort_canonical_values?: string[] | null;
};

export type {
  RawInstallRuntimeResult,
  RawInstallStepResult,
} from "./installTypes";

type RawGitBashPrerequisite = {
  available: boolean;
  path: string | null;
  install_instructions_url: string;
  install_hint: string;
};

type RawCommandAvailability = {
  command: string;
  resolved_path: string | null;
  available: boolean;
};

type RawManagedAgentPrereqs = {
  acp: RawCommandAvailability;
  mcp: RawCommandAvailability;
};

type RawRelayMember = {
  pubkey: string;
  role: string;
  added_by: string | null;
  created_at: string;
};

type RawListRelayMembersResponse = {
  members: RawRelayMember[];
};

type RawCanvasResponse = {
  content: string | null;
  updated_at: number | null;
  author: string | null;
};

type RawSetCanvasResult = {
  ok: boolean;
  event_id: string;
};

/** Error normalized from a rejected Tauri invocation with its wire payload. */
export class TauriInvokeError extends Error {
  readonly payload: unknown;

  constructor(message: string, payload: unknown) {
    super(message);
    this.name = "TauriInvokeError";
    this.payload = payload;
  }
}

function toTauriError(error: unknown): Error {
  if (error instanceof Error) {
    return error;
  }

  if (typeof error === "string") {
    return new TauriInvokeError(error, error);
  }

  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return new TauriInvokeError(error.message, error);
  }

  try {
    return new TauriInvokeError(JSON.stringify(error), error);
  } catch {
    return new TauriInvokeError("Unknown Tauri error", error);
  }
}

export async function invokeTauri<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (error) {
    // HTTP backoff lives in Rust. Do not apply its separate ApiCalls quota
    // to the WebSocket gate, but preserve the failure for the caller.
    throw toTauriError(error);
  }
}

export function fromRawFeedItem(item: RawFeedItem) {
  return {
    id: item.id,
    kind: item.kind,
    pubkey: item.pubkey,
    content: item.content,
    createdAt: item.created_at,
    channelId: item.channel_id,
    channelName: item.channel_name,
    // Canonicalize the wire `null` to undefined so FeedItem's optional
    // channelType contract holds at runtime (enrichment and the DM
    // notification filter both key off `=== undefined`).
    channelType: item.channel_type ?? undefined,
    tags: item.tags,
    category: item.category,
  };
}

function fromRawSearchHit(hit: RawSearchHit) {
  return {
    eventId: hit.event_id,
    content: hit.content,
    kind: hit.kind,
    pubkey: hit.pubkey,
    channelId: hit.channel_id,
    channelName: hit.channel_name,
    createdAt: hit.created_at,
    score: hit.score,
  };
}

export async function getPresence(pubkeys: string[]): Promise<PresenceLookup> {
  const response = await invokeTauri<RawPresenceLookup>("get_presence", {
    pubkeys,
  });

  return Object.fromEntries(
    Object.entries(response).map(([pubkey, status]) => [
      pubkey.toLowerCase(),
      status,
    ]),
  );
}

export function getDefaultRelayUrl(): Promise<string> {
  return invokeTauri<string>("get_default_relay_url");
}

export function autoConnectDefaultRelayEnabled(): Promise<boolean> {
  return invokeTauri<boolean>("auto_connect_default_relay_enabled");
}

export function isSharedIdentity(): Promise<boolean> {
  return invokeTauri<boolean>("is_shared_identity");
}

export function getRelayWsUrl(): Promise<string> {
  return invokeTauri<string>("get_relay_ws_url");
}

export function getRelayHttpUrl(): Promise<string> {
  return invokeTauri<string>("get_relay_http_url");
}

export async function addChannelMembers(
  input: AddChannelMembersInput,
): Promise<AddChannelMembersResult> {
  return invokeTauri<RawAddChannelMembersResult>("add_channel_members", input);
}

export async function removeChannelMember(
  channelId: string,
  pubkey: string,
): Promise<void> {
  await invokeTauri("remove_channel_member", { channelId, pubkey });
}

export async function changeChannelMemberRole(
  channelId: string,
  pubkey: string,
  role: string,
): Promise<void> {
  await invokeTauri("change_channel_member_role", { channelId, pubkey, role });
}

export async function joinChannel(channelId: string): Promise<void> {
  await invokeTauri("join_channel", { channelId });
}

export async function leaveChannel(channelId: string): Promise<void> {
  await invokeTauri("leave_channel", { channelId });
}

export async function getCanvas(channelId: string): Promise<CanvasResponse> {
  const response = await invokeTauri<RawCanvasResponse>("get_canvas", {
    channelId,
  });
  return {
    content: response.content,
    // Normalize absent keys to null: ensureWelcomeCanvas treats null as
    // "no canvas yet", and `undefined !== null` would make every fresh
    // channel look already-seeded.
    updatedAt: response.updated_at ?? null,
    author: response.author ?? null,
  };
}

export async function setCanvas(
  input: SetCanvasInput,
): Promise<SetCanvasResult> {
  const response = await invokeTauri<RawSetCanvasResult>("set_canvas", {
    channelId: input.channelId,
    content: input.content,
  });
  return {
    ok: response.ok,
    eventId: response.event_id,
  };
}

export async function getHomeFeed(
  input: GetHomeFeedInput = {},
): Promise<HomeFeedResponse> {
  const response = await invokeTauri<RawHomeFeedResponse>("get_feed", input);

  return {
    feed: {
      mentions: response.feed.mentions.map(fromRawFeedItem),
      needsAction: response.feed.needs_action.map(fromRawFeedItem),
      activity: response.feed.activity.map(fromRawFeedItem),
      agentActivity: response.feed.agent_activity.map(fromRawFeedItem),
    },
    meta: {
      since: response.meta.since,
      total: response.meta.total,
      generatedAt: response.meta.generated_at,
    },
  };
}

export async function searchMessages(
  input: SearchMessagesInput,
): Promise<SearchMessagesResponse> {
  const response = await invokeTauri<RawSearchResponse>("search_messages", {
    q: input.q,
    limit: input.limit,
    channelId: input.channelId,
    authors: input.authors,
    since: input.since,
    until: input.until,
  });

  return {
    hits: response.hits.map(fromRawSearchHit),
    found: response.found,
  };
}

type RawThreadCursor = {
  created_at: number;
  event_id: string;
};

type RawThreadRepliesResponse = {
  events: RelayEvent[];
  next_cursor: RawThreadCursor | null;
};

/**
 * Fetch the full reply subtree under a thread root, server-side.
 *
 * Unlike the channel timeline (which the desktop assembles from its local
 * cache), this walks `thread_metadata` on the relay, so a thread renders
 * complete even when its replies fell outside the channel cold-load window —
 * the descendant gap that made deep/old threads silently incomplete.
 *
 * Paging is forward keyset on `(createdAt, eventId)`: pass the returned
 * `nextCursor` back as `cursor` for the next page. `nextCursor` is non-null only
 * when a full page was returned. The returned `events` are raw nostr events
 * (`RelayEvent`), chronological (oldest first). These are the *replies* under
 * the root (depth >= 1); the root event itself is NOT returned (the relay query
 * keys on `root_event_id`, which a root row lacks). The caller already holds
 * the root — it is the open thread head.
 */
export async function getThreadReplies(
  rootEventId: string,
  channelId?: string | null,
  options?: {
    limit?: number;
    depthLimit?: number;
    cursor?: ThreadCursor | null;
  },
): Promise<ThreadRepliesResponse> {
  const response = await invokeTauri<RawThreadRepliesResponse>(
    "get_thread_replies",
    {
      rootEventId,
      channelId: channelId ?? null,
      limit: options?.limit ?? null,
      depthLimit: options?.depthLimit ?? null,
      cursor: options?.cursor
        ? {
            created_at: options.cursor.createdAt,
            event_id: options.cursor.eventId,
          }
        : null,
    },
  );

  return {
    events: response.events,
    nextCursor: response.next_cursor
      ? {
          createdAt: response.next_cursor.created_at,
          eventId: response.next_cursor.event_id,
        }
      : null,
  };
}

export type BlobDescriptor = {
  url: string;
  sha256: string;
  size: number;
  type: string;
  uploaded: number;
  dim?: string;
  blurhash?: string;
  thumb?: string;
  duration?: number;
  image?: string;
  /** Original filename captured client-side. */
  filename?: string;
};

export async function uploadMedia(
  filePath: string,
  isTemp: boolean,
): Promise<BlobDescriptor> {
  return invokeTauri<BlobDescriptor>("upload_media", {
    filePath,
    isTemp,
  });
}
export async function pickAndUploadMedia(
  progressId?: string,
): Promise<BlobDescriptor[]> {
  return invokeTauri<BlobDescriptor[]>("pick_and_upload_media", { progressId });
}
export async function uploadMediaBytes(
  data: number[],
  filename?: string,
  /** Correlation id for `media-upload-progress` events from the Rust side. */
  progressId?: string,
): Promise<BlobDescriptor> {
  return invokeTauri<BlobDescriptor>("upload_media_bytes", {
    data,
    filename,
    progressId,
  });
}

export { editMessage } from "@/shared/api/editMessage";

export async function deleteMessage(
  channelId: string,
  eventId: string,
): Promise<void> {
  await invokeTauri("delete_message", { channelId, eventId });
}

export async function addReaction(
  eventId: string,
  emoji: string,
  emojiUrl?: string,
): Promise<void> {
  await invokeTauri("add_reaction", { eventId, emoji, emojiUrl });
}

export async function removeReaction(
  eventId: string,
  emoji: string,
): Promise<void> {
  await invokeTauri("remove_reaction", { eventId, emoji });
}

export async function signRelayEvent(input: {
  kind: number;
  content: string;
  createdAt?: number;
  tags: string[][];
}): Promise<RelayEvent> {
  const eventJson = await invokeTauri<string>("sign_event", input);
  return JSON.parse(eventJson) as RelayEvent;
}

export async function createAuthEvent(input: {
  challenge: string;
  relayUrl: string;
}): Promise<RelayEvent> {
  const eventJson = await invokeTauri<string>("create_auth_event", input);
  return JSON.parse(eventJson) as RelayEvent;
}
function fromRawRelayAgent(agent: RawRelayAgent): RelayAgent {
  return {
    pubkey: agent.pubkey,
    ownerPubkey: agent.owner_pubkey ?? null,
    name: agent.name,
    agentType: agent.agent_type,
    channels: agent.channels,
    channelIds: agent.channel_ids ?? [],
    capabilities: agent.capabilities,
    status: agent.status,
    respondTo: agent.respond_to ?? null,
    respondToAllowlist: agent.respond_to_allowlist ?? [],
  };
}

export function fromRawManagedAgent(agent: RawManagedAgent): ManagedAgent {
  return {
    pubkey: agent.pubkey,
    name: agent.name,
    personaId: agent.persona_id,
    runtime: agent.runtime ?? null,
    teamId: agent.team_id ?? null,
    managerPubkey: agent.manager_pubkey ?? null,
    relayUrl: agent.relay_url,
    acpCommand: agent.acp_command,
    agentCommand: agent.agent_command,
    agentCommandOverride: agent.agent_command_override ?? null,
    agentArgs: agent.agent_args,
    mcpCommand: agent.mcp_command,
    turnTimeoutSeconds: agent.turn_timeout_seconds,
    idleTimeoutSeconds: agent.idle_timeout_seconds,
    maxTurnDurationSeconds: agent.max_turn_duration_seconds,
    parallelism: agent.parallelism,
    sessionPolicy: agent.session_policy ?? "channel",
    systemPrompt: agent.system_prompt,
    avatarUrl: agent.avatar_url ?? null,
    model: agent.model,
    modelSource: agent.model_source ?? null,
    provider: agent.provider ?? null,
    personaOutOfDate: agent.persona_out_of_date ?? false,
    personaOrphaned: agent.persona_orphaned ?? false,
    needsRestart: agent.needs_restart ?? false,
    restartDiff: agent.restart_diff ?? [],
    envVars: agent.env_vars ?? {},
    status: agent.status,
    pid: agent.pid,
    createdAt: agent.created_at,
    updatedAt: agent.updated_at,
    lastStartedAt: agent.last_started_at,
    lastStoppedAt: agent.last_stopped_at,
    lastExitCode: agent.last_exit_code,
    lastError: agent.last_error,
    lastErrorCode: agent.last_error_code ?? null,
    logPath: agent.log_path,
    startOnAppLaunch: agent.start_on_app_launch,
    autoRestartOnConfigChange: agent.auto_restart_on_config_change ?? true,
    backend: agent.backend,
    backendAgentId: agent.backend_agent_id,
    respondTo: agent.respond_to ?? "owner-only",
    respondToAllowlist: agent.respond_to_allowlist ?? [],
  };
}

export function fromRawAcpRuntimeCatalogEntry(
  entry: RawAcpRuntimeCatalogEntry,
): AcpRuntimeCatalogEntry {
  return {
    id: entry.id,
    label: entry.label,
    avatarUrl: entry.avatar_url,
    availability: entry.availability,
    command: entry.command,
    binaryPath: entry.binary_path,
    defaultArgs: entry.default_args,
    mcpCommand: entry.mcp_command,
    modelEnvVar: entry.model_env_var ?? null,
    providerEnvVar: entry.provider_env_var ?? null,
    thinkingEnvVar: entry.thinking_env_var ?? null,
    maxTokensEnvVar: entry.max_tokens_env_var ?? null,
    contextLimitEnvVar: entry.context_limit_env_var ?? null,
    maxRoundsEnvVar: entry.max_rounds_env_var ?? null,
    installHint: entry.install_hint,
    installInstructionsUrl: entry.install_instructions_url,
    canAutoInstall: entry.can_auto_install,
    requiresExternalCli: entry.requires_external_cli ?? false,
    underlyingCliPath: entry.underlying_cli_path,
    nodeRequired: entry.node_required,
    authStatus: entry.auth_status,
    loginHint: entry.login_hint ?? null,
    source: entry.source,
    definitionEnv: entry.definition_env ?? {},
    effortCanonicalValues: entry.effort_canonical_values ?? null,
    ...(entry.max_parallelism !== undefined && {
      maxParallelism: entry.max_parallelism,
    }),
  };
}

function fromRawCommandAvailability(
  command: RawCommandAvailability,
): CommandAvailability {
  return {
    command: command.command,
    resolvedPath: command.resolved_path,
    available: command.available,
  };
}

// ── Relay Members ────────────────────────────────────────────────────────────

function fromRawRelayMember(raw: RawRelayMember): RelayMember {
  return {
    pubkey: raw.pubkey,
    role: raw.role as RelayMemberRole,
    addedBy: raw.added_by,
    createdAt: raw.created_at,
  };
}

export async function listRelayMembers(): Promise<RelayMember[]> {
  const response =
    await invokeTauri<RawListRelayMembersResponse>("list_relay_members");
  return response.members.map(fromRawRelayMember);
}

export async function getMyRelayMembership(): Promise<RelayMember | null> {
  try {
    const raw = await invokeTauri<RawRelayMember>("get_my_relay_membership");
    return fromRawRelayMember(raw);
  } catch (error) {
    // "relay returned 404 Not Found" = not a relay member — return null so
    // the UI hides the Members tab. Re-throw real errors (network, auth, 500)
    // so React Query surfaces them.
    if (
      error instanceof Error &&
      error.message.startsWith("relay returned 404")
    ) {
      return null;
    }
    throw error;
  }
}

export async function addRelayMember(
  targetPubkey: string,
  role: string,
): Promise<void> {
  await invokeTauri("add_relay_member", { targetPubkey, role });
}

export async function removeRelayMember(targetPubkey: string): Promise<void> {
  await invokeTauri("remove_relay_member", { targetPubkey });
}

export async function changeRelayMemberRole(
  targetPubkey: string,
  newRole: string,
): Promise<void> {
  await invokeTauri("change_relay_member_role", { targetPubkey, newRole });
}

export async function listRelayAgents(): Promise<RelayAgent[]> {
  return (await invokeTauri<RawRelayAgent[]>("list_relay_agents")).map(
    fromRawRelayAgent,
  );
}

export async function listManagedAgents(): Promise<ManagedAgent[]> {
  return (await invokeTauri<RawManagedAgent[]>("list_managed_agents")).map(
    fromRawManagedAgent,
  );
}
export async function createManagedAgent(input: CreateManagedAgentInput) {
  const response = await invokeTauri<RawCreateManagedAgentResponse>(
    "create_managed_agent",
    {
      input: {
        name: input.name,
        personaId: input.personaId,
        teamId: input.teamId,
        relayUrl: input.relayUrl,
        acpCommand: input.acpCommand,
        agentCommand: input.agentCommand,
        harnessOverride: input.harnessOverride ?? false,
        agentArgs: input.agentArgs,
        mcpCommand: input.mcpCommand,
        turnTimeoutSeconds: input.turnTimeoutSeconds,
        idleTimeoutSeconds: input.idleTimeoutSeconds,
        maxTurnDurationSeconds: input.maxTurnDurationSeconds,
        parallelism: input.parallelism,
        systemPrompt: input.systemPrompt,
        avatarUrl: input.avatarUrl,
        model: input.model,
        provider: input.provider,
        envVars: input.envVars ?? {},
        spawnAfterCreate: input.spawnAfterCreate,
        startOnAppLaunch: input.startOnAppLaunch,
        backend: input.backend,
        respondTo: input.respondTo,
        respondToAllowlist: input.respondToAllowlist,
        relayMesh: input.relayMesh,
      },
    },
  );
  return {
    agent: fromRawManagedAgent(response.agent),
    privateKeyNsec: response.private_key_nsec,
    profileSyncError: response.profile_sync_error,
    spawnError: response.spawn_error,
  };
}

export async function deleteManagedAgent(
  pubkey: string,
  forceRemoteDelete?: boolean,
): Promise<void> {
  await invokeTauri("delete_managed_agent", {
    pubkey,
    forceRemoteDelete: forceRemoteDelete ?? null,
  });
}

export async function getManagedAgentLog(pubkey: string, lineCount?: number) {
  const response = await invokeTauri<RawManagedAgentLog>(
    "get_managed_agent_log",
    {
      pubkey,
      lineCount,
    },
  );

  return {
    content: response.content,
    logPath: response.log_path,
  };
}

export async function discoverGitBashPrerequisite(): Promise<GitBashPrerequisite | null> {
  const prerequisite = await invokeTauri<RawGitBashPrerequisite | null>(
    "discover_git_bash_prerequisite",
  );
  return (
    prerequisite && {
      available: prerequisite.available,
      path: prerequisite.path,
      installInstructionsUrl: prerequisite.install_instructions_url,
      installHint: prerequisite.install_hint,
    }
  );
}

/** Input shape for creating or updating a custom harness. */
export type HarnessDefinitionInput = {
  id: string;
  label: string;
  command: string;
  args?: string[];
  env?: Record<string, string>;
  installInstructionsUrl?: string;
  installHint?: string;
};

/** Save (create or overwrite) a custom harness definition. Returns the catalog entry. */
export async function saveCustomHarness(
  definition: HarnessDefinitionInput,
  originalId?: string,
): Promise<AcpRuntimeCatalogEntry> {
  const raw = await invokeTauri<RawAcpRuntimeCatalogEntry>(
    "save_custom_harness",
    {
      definition: {
        id: definition.id,
        label: definition.label,
        command: definition.command,
        args: definition.args ?? [],
        env: definition.env ?? {},
        installInstructionsUrl: definition.installInstructionsUrl ?? "",
        installHint: definition.installHint ?? "",
      },
      originalId: originalId ?? null,
    },
  );
  return fromRawAcpRuntimeCatalogEntry(raw);
}

/** Delete a custom harness definition by id. No-op if already gone. */
export async function deleteCustomHarness(id: string): Promise<void> {
  await invokeTauri<void>("delete_custom_harness", { id });
}

export async function installAcpRuntime(
  runtimeId: string,
): Promise<InstallRuntimeResult> {
  const raw = await invokeTauri<RawInstallRuntimeResult>(
    "install_acp_runtime",
    { runtimeId },
  );
  return fromRawInstallRuntimeResult(raw);
}

export async function discoverManagedAgentPrereqs(input: {
  acpCommand?: string;
  mcpCommand?: string;
}) {
  const response = await invokeTauri<RawManagedAgentPrereqs>(
    "discover_managed_agent_prereqs",
    {
      input: {
        acpCommand: input.acpCommand,
        mcpCommand: input.mcpCommand,
      },
    },
  );

  return {
    acp: fromRawCommandAvailability(response.acp),
    mcp: fromRawCommandAvailability(response.mcp),
  };
}

// ── Model discovery ───────────────────────────────────────────────────────────

export async function getAgentModels(pubkey: string) {
  return invokeTauri<AgentModelsResponse>("get_agent_models", { pubkey });
}

export async function getAgentConfigSurface(
  pubkey: string,
): Promise<RuntimeConfigSurface> {
  return invokeTauri<RuntimeConfigSurface>("get_agent_config_surface", {
    pubkey,
  });
}

export async function putAgentSessionConfig(
  pubkey: string,
  payload: unknown,
): Promise<void> {
  return invokeTauri<void>("put_agent_session_config", { pubkey, payload });
}

/** File-layer config for a runtime (e.g. `~/.config/goose/config.yaml`). */
export type RuntimeFileConfigSubset = {
  /** Provider set in the harness config file. */
  provider: string | null;
  /** Model set in the harness config file. */
  model: string | null;
  /** Credential env key names whose values are present in the file config. */
  satisfiedEnvKeys: string[];
};

/**
 * Get the file-layer config for a runtime so dialogs can show "Set in goose config" instead of
 * surfacing a false required-field marker. Returns `null` when unavailable or unparseable.
 */
export async function getRuntimeFileConfig(
  runtimeId: string,
): Promise<RuntimeFileConfigSubset | null> {
  return invokeTauri<RuntimeFileConfigSubset | null>(
    "get_runtime_file_config",
    {
      runtimeId,
    },
  );
}

/**
 * Return the key names of all non-empty baked build env vars. Internal (Block) builds bake
 * provider credentials into the binary at compile time; this returns *key names only* (never
 * values) so dialogs treat them as satisfied without exposing secrets. OSS builds return [].
 */
export async function getBakedBuildEnvKeys(): Promise<string[]> {
  return invokeTauri<string[]>("get_baked_build_env_keys");
}

/**
 * A single baked build env entry.
 *
 * The value is already masked in Rust for secret keys (keys not in the
 * explicit safe-to-reveal allowlist: `BUZZ_AGENT_PROVIDER`, `BUZZ_AGENT_MODEL`,
 * `DATABRICKS_HOST`, `DATABRICKS_MODEL`, `DATABRICKS_MODEL_FILTER`). Non-allowlisted keys have their
 * values replaced with `••••••`. Non-secret values are shown as-is.
 * Empty-value keys are filtered out.
 */
export type BakedEnvEntry = {
  key: string;
  /** Display value — real value or `••••••` for masked keys. */
  value: string;
  /** `true` when the value was replaced by the mask placeholder in Rust. */
  masked: boolean;
};

/**
 * Return the baked build env entries with values shown (masked where
 * appropriate) for display in the Agent defaults card.
 *
 * Provider and model arrive as `BUZZ_AGENT_PROVIDER` / `BUZZ_AGENT_MODEL`
 * keys and are included in the list alongside other baked vars.
 *
 * OSS builds return an empty array — the baked-env section is hidden.
 */
export async function getBakedBuildEnv(): Promise<BakedEnvEntry[]> {
  return invokeTauri<BakedEnvEntry[]>("get_baked_build_env");
}

type RawUpdateManagedAgentResponse = {
  agent: RawManagedAgent;
  profile_sync_error: string | null;
};

export async function updateManagedAgent(
  input: UpdateManagedAgentInput,
): Promise<{ agent: ManagedAgent; profileSyncError: string | null }> {
  const response = await invokeTauri<RawUpdateManagedAgentResponse>(
    "update_managed_agent",
    { input },
  );
  return {
    agent: fromRawManagedAgent(response.agent),
    profileSyncError: response.profile_sync_error,
  };
}

// ── Backend provider discovery ────────────────────────────────────────────────

export async function discoverBackendProviders(): Promise<
  BackendProviderCandidate[]
> {
  return invokeTauri<BackendProviderCandidate[]>("discover_backend_providers");
}

export async function probeBackendProvider(
  binaryPath: string,
): Promise<BackendProviderProbeResult> {
  return invokeTauri<BackendProviderProbeResult>("probe_backend_provider", {
    binaryPath,
  });
}

// ── NIP-44 encrypt-to-self ───────────────────────────────────────────────────

export async function nip44EncryptToSelf(plaintext: string): Promise<string> {
  return invokeTauri<string>("nip44_encrypt_to_self", { plaintext });
}

export async function nip44DecryptFromSelf(
  ciphertext: string,
): Promise<string> {
  return invokeTauri<string>("nip44_decrypt_from_self", { ciphertext });
}

export async function startPairing(): Promise<string> {
  return invokeTauri<string>("start_pairing");
}

export async function confirmPairingSas(): Promise<void> {
  await invokeTauri("confirm_pairing_sas");
}

export async function cancelPairing(): Promise<void> {
  await invokeTauri("cancel_pairing");
}

// Validate a candidate repos dir without mutating the filesystem. Rejects
// with a human-readable reason; resolves for a valid or empty path.
export async function validateReposDir(dir: string): Promise<void> {
  await invokeTauri("validate_repos_dir", { dir });
}

export const setPreventSleepActive = (active: boolean) =>
  invokeTauri("set_prevent_sleep_active", { active });

/** Returns true on macOS, Windows, and Linux AppImage installs.
 *  Returns false on Linux non-AppImage packages (e.g. .deb) where
 *  Tauri's updater cannot swap the binary. */
export function isAutoUpdateSupported(): Promise<boolean> {
  return invokeTauri<boolean>("is_auto_update_supported");
}

// ── Break / Start (stop every agent at once, then bring them back) ──────────

/** One agent/community pair a Break stopped. */
export type PausedPair = {
  pubkey: string;
  relayUrl: string;
  /** Display name captured at the break, so a restored UI can still name it. */
  name: string | null;
  /**
   * What this agent was last observed doing, captured at the pause. Read from
   * the archived turn rather than from the agent's configuration, so it says
   * what actually ran — which stops matching the configuration as soon as a
   * cost profile or a failover retiers the team while it is paused.
   *
   * All four are null for an agent with no archived turn in the last week.
   * Absent means absent; nothing here is inferred.
   */
  runtime: string | null;
  model: string | null;
  lastChannelId: string | null;
  /** Unix seconds of that turn. */
  lastTurnAt: number | null;
};

/** The durable record a Break leaves for the next Start. */
export type BreakRecord = {
  /** RFC3339 instant of the break, or null when nothing is paused. */
  pausedAt: string | null;
  pairs: PausedPair[];
};

/** One agent a within-provider model move would change. */
export type ReportedModelMove = {
  name: string;
  fromModel: string;
  toModel: string;
};

/** What a model-level failover would do, and why. */
export type ModelFailoverReport = {
  /** Empty when nothing is being moved; `summary` then says why. */
  moves: ReportedModelMove[];
  summary: string;
};

/** What a within-provider model move would do right now. Read-only. */
export async function modelFailoverPlan(): Promise<ModelFailoverReport> {
  return invokeTauri<ModelFailoverReport>("model_failover_plan");
}

/**
 * Carry out the move. The backend re-plans against fresh limits rather than
 * trusting the plan shown, since a window may have reset in between.
 */
export async function applyModelFailover(): Promise<ModelFailoverReport> {
  return invokeTauri<ModelFailoverReport>("apply_model_failover");
}

/** What happened when a handover note was asked for. */
export type HandoverResult = {
  /** True when this call wrote the file; false when one was already there. */
  created: boolean;
  /** Why nothing could be done, when that is the answer. */
  error: string | null;
};

/**
 * Create a handover note in the channel's folder, never overwriting one.
 *
 * An existing handover is the team's account of what is in flight and what
 * must not be built upon; it is reported as not created rather than replaced.
 */
export async function ensureChannelHandover(
  channelId: string,
  contents: string,
): Promise<HandoverResult> {
  return invokeTauri<HandoverResult>("ensure_channel_handover", {
    channelId,
    contents,
  });
}

/** One agent that is out of runway, and who stands in. */
export type LeaveEntry = {
  pubkey: string;
  name: string;
  /** The badge line, already written to be read. */
  label: string;
  /** Unix seconds of the return, or null when nothing counts down to it. */
  returnsAt: number | null;
  /** Who is covering, or null when nobody can. */
  cover: string | null;
  /**
   * Why that cover. Present on a suggestion and on a refusal, absent when the
   * owner designated the cover — so a suggestion never reads as a decision.
   */
  coverNote: string | null;
};

/** Who on the roster is out of runway right now, per the providers' own windows. */
export async function agentRollCall(): Promise<LeaveEntry[]> {
  return invokeTauri<LeaveEntry[]>("agent_roll_call");
}

/** A pair that refused to stop or start, with the reason. */
export type PairFailure = {
  pubkey: string;
  relayUrl: string;
  error: string;
};

/** Outcome of a Break or a Start — partial success is reported, not hidden. */
export type BreakOutcome = {
  succeeded: number;
  failures: PairFailure[];
  record: BreakRecord;
};

/** Stop every running agent and record what was stopped. */
export async function breakManagedAgents(): Promise<BreakOutcome> {
  return invokeTauri<BreakOutcome>("break_managed_agents");
}

/** Start exactly the agents the last Break recorded. */
export async function resumeManagedAgents(): Promise<BreakOutcome> {
  return invokeTauri<BreakOutcome>("resume_managed_agents");
}

/** What is currently paused, for the button's label and count. */
export async function managedAgentsBreakState(): Promise<BreakRecord> {
  return invokeTauri<BreakRecord>("managed_agents_break_state");
}

/**
 * A team-wide model policy. `economy` is the cheapest tier everywhere,
 * `balanced` keeps strong models only where the team's direction is decided,
 * `max` is what an unconstrained team costs.
 */
export type CostProfile = "economy" | "balanced" | "max";

/** What applying a profile actually changed, counted by kind. */
export type CostProfileOutcome = {
  /** Agents whose model moved to a different tier. */
  retiered: number;
  /** Agents on a single-model runtime: concurrency changed, model could not. */
  cappedOnly: number;
  /** Agents already matching the profile. */
  unchanged: number;
  /** Agents with no recognized runtime, left untouched. */
  skipped: number;
  /** A few names that moved tier, for the confirmation message. */
  examples: string[];
};

/** Apply a cost profile to every managed agent. */
export async function applyCostProfile(
  profile: CostProfile,
): Promise<CostProfileOutcome> {
  return invokeTauri<CostProfileOutcome>("apply_cost_profile", { profile });
}

/** The profile the store already matches, or null when it matches none. */
export async function currentCostProfile(): Promise<CostProfile | null> {
  return invokeTauri<CostProfile | null>("current_cost_profile");
}

/** One provider limit window, as that provider describes it. */
export type LimitWindow = {
  /** The provider's own name for the window, e.g. "Session" or "Week · Fable". */
  label: string;
  /** Percent of the window consumed, 0-100. */
  usedPercent: number;
  /** When it resets, in the provider's own wording. */
  resets: string | null;
  /** Unix seconds, when the provider gave an instant rather than prose. */
  resetsAt: number | null;
  /**
   * How long the whole window is, in minutes. Codex states it; Claude's is
   * derived from the CLI's own wording. Null when neither says, which is what
   * stops a forecast being drawn over a window of unknown length.
   */
  windowMinutes: number | null;
  /**
   * The window already reset, so `usedPercent` describes a window that no
   * longer exists. Only reachable for a runtime read from its own log: Codex
   * writes a limit when it takes a turn, so an idle Codex leaves its last
   * reading behind after the window rolls over.
   */
  stale: boolean;
};

/** Every limit window one runtime reports, or why it reports none. */
export type LimitGroup = {
  runtime: string;
  label: string;
  windows: LimitWindow[];
  plan: string | null;
  note: string | null;
  /** Set when this runtime could not be read; its windows are then empty. */
  unavailable: string | null;
  /** Age of the reading in seconds, when it came from a file the runtime wrote. */
  ageSeconds: number | null;
};

/**
 * The real provider limits — the ceilings that actually stop a run, read from
 * each runtime's own reporting rather than estimated from spend.
 *
 * `refresh` skips the backend's cache; the Claude read costs one small request,
 * so leave it off for routine polling.
 */
export async function getProviderLimits(
  refresh?: boolean,
): Promise<LimitGroup[]> {
  return invokeTauri<LimitGroup[]>("get_provider_limits", { refresh });
}

/** What the governor does when a window crosses the threshold. */
export type GovernorAction = "notify" | "economy" | "break";

/** The owner's governor settings. Ships disabled. */
export type GovernorConfig = {
  enabled: boolean;
  /** Percent of a window at which to act. */
  thresholdPercent: number;
  action: GovernorAction;
};

/** Settings plus what the governor has already done. */
export type GovernorRecord = {
  config: GovernorConfig;
  /** Window occurrences already acted on, so one crossing acts once. */
  handled: string[];
  lastFiredAt: string | null;
  lastReason: string | null;
  lastAction: GovernorAction | null;
  lastOutcome: string | null;
};

/** Read the governor's settings and history. */
export async function budgetGovernorState(): Promise<GovernorRecord> {
  return invokeTauri<GovernorRecord>("budget_governor_state");
}

/**
 * Replace the governor's settings.
 *
 * Changing them clears the handled list, so a stricter threshold applies to
 * the window you are in now rather than only to the next one.
 */
export async function setBudgetGovernor(
  config: GovernorConfig,
): Promise<GovernorRecord> {
  return invokeTauri<GovernorRecord>("set_budget_governor", { config });
}

/**
 * Session recycler settings.
 *
 * An ACP session accumulates: every turn re-sends the conversation so far, so
 * the same piece of work costs more the older the session it happens in.
 * Recycling restarts an agent, which drops its context back to the system
 * prompt. Ships disabled.
 */
export type RecyclerConfig = {
  enabled: boolean;
  /** Recycle a session once it has been up this long. */
  maxSessionHours: number;
  /** ...but only after this long with no turn, so work is never cut off. */
  minIdleMinutes: number;
};

/** Read the recycler settings. */
export async function sessionRecyclerConfig(): Promise<RecyclerConfig> {
  return invokeTauri<RecyclerConfig>("session_recycler_config");
}

/** Replace the recycler settings. */
export async function setSessionRecycler(
  config: RecyclerConfig,
): Promise<RecyclerConfig> {
  return invokeTauri<RecyclerConfig>("set_session_recycler", { config });
}

/** How much trouble a shared-identity overlap is causing right now. */
export type ConflictSeverity = "none" | "warning" | "critical";

/** What another Buzz-family install shares with this one. */
export type ConflictReport = {
  bundleId: string;
  /** Agent identities both apps hold. */
  sharedAgents: number;
  /** Of those, the ones the sibling currently has running. */
  runningSharedAgents: number;
  severity: ConflictSeverity;
  sampleNames: string[];
};

/**
 * Buzz-family installs that hold the same agent keys as this one.
 *
 * Two apps signing as one identity on the same relay interleave their turns
 * and burn two subscriptions on one conversation. Read-only: the sibling's
 * store is compared, never modified, and its private keys are never read.
 */
export async function siblingConflicts(): Promise<ConflictReport[]> {
  return invokeTauri<ConflictReport[]>("sibling_conflicts");
}

/** Wave scheduler settings: how many agents may run at once. */
export type WaveConfig = {
  enabled: boolean;
  /** How many agents may be running at once. */
  concurrentAgents: number;
  /** An agent with no turn for this long is treated as finished. */
  idleMinutes: number;
};

/** Read the wave scheduler settings. */
export async function waveSchedulerConfig(): Promise<WaveConfig> {
  return invokeTauri<WaveConfig>("wave_scheduler_config");
}

/** Replace the wave scheduler settings. */
export async function setWaveScheduler(
  config: WaveConfig,
): Promise<WaveConfig> {
  return invokeTauri<WaveConfig>("set_wave_scheduler", { config });
}

/** One declared channel output and what is on disk for it. */
export type Deliverable = {
  /** Path as declared, relative to the channel's folder. */
  path: string;
  present: boolean;
  sizeBytes: number | null;
  modifiedAt: number | null;
  /** Why this target could not be checked, if it could not. */
  error: string | null;
};

/** A channel's declared outputs and how far along they are. */
export type DeliverableReport = {
  targets: Deliverable[];
  /** Targets that exist and are not empty. */
  produced: number;
  /** Targets that exist but contain nothing yet. */
  empty: number;
  total: number;
  /** True when the channel has no folder bound to look in. */
  noWorkspace: boolean;
};

/**
 * What this channel is supposed to produce, and whether it has.
 *
 * A filesystem check only: it says a file exists and how big it is, never that
 * its contents are any good.
 */
export async function channelDeliverables(
  channelId: string,
): Promise<DeliverableReport> {
  return invokeTauri<DeliverableReport>("channel_deliverables", { channelId });
}

/** Replace this channel's declared outputs. */
export async function setChannelDeliverables(
  channelId: string,
  targets: string[],
): Promise<string[]> {
  return invokeTauri<string[]>("set_channel_deliverables", {
    channelId,
    targets,
  });
}
