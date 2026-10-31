import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

async function setup(page: Page) {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.evaluate(() => {
    type Member = { pubkey: string; role: string; capabilities: string[] };
    type Team = {
      generation: number;
      coordinator_pubkey: string | null;
      members: Member[];
      paused: boolean;
    };
    const bridge = window as typeof window & {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      __teamFailure?: boolean;
      __teamLoseReply?: boolean;
      __teamDelay?: boolean;
      __teamResolve?: () => void;
      __teamBump?: () => void;
      __teamWrites: Record<string, unknown>[];
      __teamDeleteCalls: string[];
      __inspectionCalls: Record<string, unknown>[];
      __inspectionWrongScope?: boolean;
      __inspectionFailure?: boolean;
      __workspaceFailure?: boolean;
      __currentBindingGeneration?: number;
      __pendingNativeEdits?: {
        operation_id: string;
        channel_id: string;
        relative_path: string;
        before_sha256: string;
        after_sha256: string;
      }[];
      __nativeObservedHash?: string;
      __nativeReceiptReadFailure?: boolean;
      __nativeReconciliationCalls: Record<string, unknown>[];
      __coordinatorCalls: Record<string, unknown>[];
      __agentProvisionCalls: string[];
      __coordinatorMode?: "opt_in" | "started" | "stale" | "lost_reply";
    };
    const original = bridge.__TAURI_INTERNALS__.invoke.bind(
      bridge.__TAURI_INTERNALS__,
    );
    const teams = new Map<string, Team>();
    const available = [
      {
        pubkey: "ab".repeat(32),
        display_name: "CORTEX",
        runtime_id: "codex",
        local: true,
      },
      {
        pubkey: "cd".repeat(32),
        display_name: "Byte",
        runtime_id: "claude",
        local: true,
      },
    ];
    bridge.__teamWrites = [];
    bridge.__teamDeleteCalls = [];
    bridge.__inspectionCalls = [];
    bridge.__nativeReconciliationCalls = [];
    bridge.__coordinatorCalls = [];
    bridge.__agentProvisionCalls = [];
    bridge.__TAURI_INTERNALS__.invoke = async (command, payload) => {
      if (
        /create_managed_agent|deploy_managed_agent|import.*agent/.test(command)
      )
        bridge.__agentProvisionCalls.push(command);
      if (/delete.*agent|remove_managed_agent/.test(command))
        bridge.__teamDeleteCalls.push(command);
      if (command === "start_channel_project_coordinator" && payload) {
        const key = `${payload.expectedRelay}:${payload.expectedOwner}:${payload.channelId}`;
        const current = teams.get(key);
        bridge.__coordinatorCalls.push(payload);
        if (
          !current ||
          current.paused ||
          current.generation !== payload.expectedTeamGeneration ||
          payload.expectedBindingGeneration !==
            (bridge.__currentBindingGeneration ?? 1) ||
          current.coordinator_pubkey !== payload.coordinatorPubkey
        )
          throw new Error(
            "Project coordinator authority changed; nothing started",
          );
        if (bridge.__coordinatorMode === "lost_reply")
          throw new Error("Startup outcome uncertain");
        const started = bridge.__coordinatorMode === "started";
        const optIn = bridge.__coordinatorMode === "opt_in";
        return {
          status: started ? "started" : "blocked",
          transport: optIn ? "codex_reasoning_broker_v1" : "native",
          binding_generation: payload.expectedBindingGeneration,
          team_generation:
            current.generation + (bridge.__coordinatorMode === "stale" ? 1 : 0),
          coordinator_pubkey: current.coordinator_pubkey,
          requires_explicit_opt_in: optIn,
          authentication_required:
            optIn && payload.explicitBridgeOptIn === true,
          blockers: started
            ? []
            : [
                {
                  code: optIn
                    ? payload.explicitBridgeOptIn
                      ? "scoped_authentication_required"
                      : "explicit_bridge_opt_in_required"
                    : "coordinator_not_managed_locally",
                  message: optIn
                    ? payload.explicitBridgeOptIn
                      ? "Scoped authentication is missing"
                      : "Explicit isolated bridge opt-in is required"
                    : "Coordinator is not managed on this device",
                },
              ],
          runtime_receipt: started
            ? { pid: 12345, started_at: 1789603200 }
            : null,
          execution_ready: false,
        };
      }
      if (command === "get_pending_project_edits") {
        if (bridge.__nativeReceiptReadFailure)
          throw new Error("Edit receipt verification unavailable");
        return bridge.__pendingNativeEdits ?? [];
      }
      if (command === "reconcile_project_edit" && payload) {
        if (
          payload.confirmedWriterStopped !== true ||
          !bridge.__pendingNativeEdits?.some(
            (edit) => edit.operation_id === payload.operationId,
          )
        )
          throw new Error("Stop and inspect the original writer first");
        if (payload.expectedCurrentSha256 !== bridge.__nativeObservedHash)
          throw new Error(
            "Current file differs from inspected hash; nothing reconciled",
          );
        bridge.__nativeReconciliationCalls.push(payload);
        bridge.__pendingNativeEdits = bridge.__pendingNativeEdits.filter(
          (edit) => edit.operation_id !== payload.operationId,
        );
        return null;
      }
      if (command === "get_channel_workspace") {
        if (bridge.__workspaceFailure)
          throw new Error("Workspace verification unavailable");
        return {
          pending_provider_attempts: [],
          checked_at: 1789603200,
          binding: {
            channel_id: payload?.channelId,
            generation: bridge.__currentBindingGeneration ?? 1,
            root: "/synthetic/project",
          },
          folder_readable: true,
          folder_error: null,
          execution_ready: false,
          execution_blocker: "Live runtime has not been verified",
        };
      }
      if (command === "inspect_channel_project" && payload) {
        const key = `${payload.expectedRelay}:${payload.expectedOwner}:${payload.channelId}`;
        const current = teams.get(key);
        if (
          !current ||
          current.paused ||
          current.generation !== payload.expectedTeamGeneration
        )
          throw new Error("Project authority is paused or stale");
        const coordinator = current.members.find(
          (member) => member.pubkey === current.coordinator_pubkey,
        );
        if (
          !coordinator?.capabilities.includes("read") ||
          !coordinator.capabilities.includes("run_checks")
        )
          throw new Error("Coordinator needs read and run_checks permissions");
        bridge.__inspectionCalls.push(payload);
        if (bridge.__inspectionFailure)
          throw new Error("Sandbox execution failed; no evidence produced");
        return {
          channel_id: bridge.__inspectionWrongScope
            ? "another-channel"
            : payload.channelId,
          binding_generation: 1,
          team_generation: current.generation,
          checked_at: 1789603200,
          evidence: {
            actual_cwd: "/synthetic/project",
            revision: "ab".repeat(20),
            branch: "synthetic-feature",
            dirty: true,
            runtime: "native-read-only-checker-v1",
          },
        };
      }
      if (
        command === "get_channel_project_team" ||
        command === "set_channel_project_team"
      ) {
        const key = `${payload?.expectedRelay}:${payload?.expectedOwner}:${payload?.channelId}`;
        if (!teams.has(key))
          teams.set(key, {
            generation: 0,
            coordinator_pubkey: null,
            members: [],
            paused: true,
          });
        const current = teams.get(key) as Team;
        bridge.__teamBump = () => {
          current.generation += 1;
        };
        if (command === "get_channel_project_team" && bridge.__teamFailure)
          throw new Error("Team verification unavailable");
        if (command === "set_channel_project_team" && payload) {
          if (
            payload.expectedBindingGeneration !==
            (bridge.__currentBindingGeneration ?? 1)
          )
            throw new Error("stale project folder");
          if (payload.expectedGeneration !== current.generation)
            throw new Error("stale team");
          if (bridge.__teamDelay)
            await new Promise<void>((resolve) => {
              bridge.__teamResolve = resolve;
            });
          bridge.__teamWrites.push(payload);
          current.generation += 1;
          current.coordinator_pubkey = payload.coordinatorPubkey as
            | string
            | null;
          current.members = payload.members as Member[];
          current.paused = Boolean(payload.paused);
          if (bridge.__teamLoseReply) {
            bridge.__teamLoseReply = false;
            throw new Error("Save outcome uncertain. Refresh before retrying.");
          }
        }
        return {
          ...current,
          binding_generation: bridge.__currentBindingGeneration ?? 1,
          members: current.members.map((member) => ({
            ...member,
            display_name: available.find(
              (agent) => agent.pubkey === member.pubkey,
            )?.display_name,
          })),
          available_agents: available,
          execution_blockers: ["Live runtime has not been verified"],
        };
      }
      return original(command, payload);
    };
  });
  await page
    .getByRole("button", { name: "Channel local project", exact: true })
    .click();
  await page.getByRole("tab", { name: "Team and permissions" }).click();
  const team = page.getByRole("region", { name: "Project team", exact: true });
  await expect(team.getByLabel("Project coordinator")).toBeVisible();
  return team;
}

async function saveCoordinator(page: Page) {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await team.getByRole("checkbox", { name: "Pause project work" }).uncheck();
  await team.getByRole("button", { name: "Save project permissions" }).click();
  return team;
}

test("startup is scoped to saved authority and never recreates an unmanaged coordinator", async ({
  page,
}) => {
  const team = await setup(page);
  const start = team.getByRole("button", {
    name: "Start project coordinator",
    exact: true,
  });
  await expect(start).toBeDisabled();
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await team.getByRole("checkbox", { name: "Pause project work" }).uncheck();
  await expect(start).toBeDisabled();
  await team.getByRole("button", { name: "Save project permissions" }).click();
  await start.click();
  await expect(
    team.getByText("Coordinator is not managed on this device", {
      exact: true,
    }),
  ).toBeVisible();
  const state = await page.evaluate(() => {
    const bridge = window as typeof window & {
      __coordinatorCalls: Record<string, unknown>[];
      __agentProvisionCalls: string[];
    };
    return {
      calls: bridge.__coordinatorCalls,
      provisions: bridge.__agentProvisionCalls,
    };
  });
  expect(state.calls).toEqual([
    {
      channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
      expectedOwner: "deadbeef".repeat(8),
      expectedRelay: expect.any(String),
      expectedBindingGeneration: 1,
      expectedTeamGeneration: 1,
      coordinatorPubkey: "ab".repeat(32),
      explicitBridgeOptIn: false,
    },
  ]);
  expect(state.provisions).toEqual([]);
});

test("backend-required bridge opt-in is explicit and missing authentication stays blocked", async ({
  page,
}) => {
  const team = await saveCoordinator(page);
  await page.evaluate(() => {
    (
      window as typeof window & { __coordinatorMode: string }
    ).__coordinatorMode = "opt_in";
  });
  const start = team.getByRole("button", {
    name: "Start project coordinator",
    exact: true,
  });
  await start.click();
  const optIn = team.getByRole("checkbox", {
    name: /Allow the isolated Codex project bridge/,
  });
  await expect(optIn).not.toBeChecked();
  await optIn.check();
  await start.click();
  await expect(
    team.getByText("Scoped authentication is missing", { exact: true }),
  ).toBeVisible();
  await expect(
    team.getByText(/Owner-approved sign-in to the separate project scope/),
  ).toBeVisible();
  const calls = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __coordinatorCalls: Record<string, unknown>[];
        }
      ).__coordinatorCalls,
  );
  expect(calls.map((call) => call.explicitBridgeOptIn)).toEqual([false, true]);
  await expect(team.getByText(/Coordinator process started\./)).toHaveCount(0);
});

test("startup receipt is not readiness and is cleared by changed permissions", async ({
  page,
}) => {
  const team = await saveCoordinator(page);
  await page.evaluate(() => {
    (
      window as typeof window & { __coordinatorMode: string }
    ).__coordinatorMode = "started";
  });
  const start = team.getByRole("button", {
    name: "Start project coordinator",
    exact: true,
  });
  await start.click();
  await expect(team.getByText(/Coordinator process started\./)).toBeVisible();
  await expect(
    team.getByText(/not proof that the agent has answered/),
  ).toBeVisible();
  await expect(start).toBeDisabled();
  await team.getByRole("checkbox", { name: "Edit project files" }).check();
  await expect(team.getByText(/Coordinator process started\./)).toHaveCount(0);
  await expect(start).toBeDisabled();
});

test("foreign startup receipt and stale authority never produce confirmation", async ({
  page,
}) => {
  const team = await saveCoordinator(page);
  await page.evaluate(() => {
    (
      window as typeof window & { __coordinatorMode: string }
    ).__coordinatorMode = "stale";
  });
  const start = team.getByRole("button", {
    name: "Start project coordinator",
    exact: true,
  });
  await start.click();
  const startup = team.getByRole("region", {
    name: "Coordinator startup",
    exact: true,
  });
  await expect(startup.getByRole("alert")).toContainText(
    "Startup receipt is invalid or stale",
  );
  await expect(startup.getByText(/Coordinator process started\./)).toHaveCount(
    0,
  );
  await page.evaluate(() => {
    (window as typeof window & { __teamBump: () => void }).__teamBump();
  });
  await start.click();
  await expect(startup.getByRole("alert")).toContainText(
    "authority changed; nothing started",
  );
});

test("lost startup reply is surfaced without automatic duplicate invocation", async ({
  page,
}) => {
  const team = await saveCoordinator(page);
  await page.evaluate(() => {
    (
      window as typeof window & { __coordinatorMode: string }
    ).__coordinatorMode = "lost_reply";
  });
  await team
    .getByRole("button", { name: "Start project coordinator", exact: true })
    .click();
  const startup = team.getByRole("region", {
    name: "Coordinator startup",
    exact: true,
  });
  await expect(startup.getByRole("alert")).toContainText(
    "Startup outcome uncertain",
  );
  await expect(startup.getByRole("alert")).toContainText(
    "no automatic retry was made",
  );
  await team.getByRole("button", { name: "Refresh team", exact: true }).click();
  const calls = await page.evaluate(
    () =>
      (window as typeof window & { __coordinatorCalls: unknown[] })
        .__coordinatorCalls,
  );
  expect(calls).toHaveLength(1);
});

test("owner selects existing coordinator and scoped grants without starting agents", async ({
  page,
}) => {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await team
    .getByRole("checkbox", { name: "Manage this project team" })
    .check();
  await team.getByRole("checkbox", { name: "Pause project work" }).uncheck();
  await team.getByLabel("Existing agent to add").selectOption("cd".repeat(32));
  await team.getByRole("button", { name: "Add to project team" }).click();
  const byte = team.getByRole("group", { name: "Byte", exact: true });
  await byte.getByRole("checkbox", { name: "Edit project files" }).check();
  expect(
    await page.evaluate(
      () =>
        (window as typeof window & { __teamWrites: unknown[] }).__teamWrites
          .length,
    ),
  ).toBe(0);
  await team.getByRole("button", { name: "Save project permissions" }).click();
  await expect(team.getByText(/No unsaved changes/)).toBeVisible();
  await byte.getByRole("button", { name: "Release from project" }).click();
  await team.getByRole("button", { name: "Save project permissions" }).click();
  await expect(byte).not.toBeVisible();
  const result = await page.evaluate(() => {
    const bridge = window as typeof window & {
      __teamWrites: Record<string, unknown>[];
      __teamDeleteCalls: string[];
    };
    return { writes: bridge.__teamWrites, deletions: bridge.__teamDeleteCalls };
  });
  expect(result.deletions).toEqual([]);
  expect(result.writes).toHaveLength(2);
  expect(result.writes[0]).toMatchObject({
    expectedGeneration: 0,
    expectedBindingGeneration: 1,
    expectedOwner: "deadbeef".repeat(8),
    channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
    coordinatorPubkey: "ab".repeat(32),
  });
  expect(result.writes[0].members).toEqual([
    {
      pubkey: "ab".repeat(32),
      role: "coordinator",
      capabilities: ["read", "manage_team"],
    },
    { pubkey: "cd".repeat(32), role: "member", capabilities: ["read", "edit"] },
  ]);
  expect(result.writes[1]).toMatchObject({ expectedGeneration: 1 });
  await expect(
    team.getByText("Live runtime has not been verified"),
  ).toBeVisible();
});

test("switching project sections preserves unsaved permissions", async ({
  page,
}, testInfo) => {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await team.getByRole("checkbox", { name: "Pause project work" }).check();
  await page.getByRole("tab", { name: "Workspace", exact: true }).click();
  await expect(team).not.toBeVisible();
  await page.getByRole("tab", { name: "Team and permissions" }).click();
  await expect(team.getByLabel("Project coordinator")).toHaveValue(
    "ab".repeat(32),
  );
  await expect(
    team.getByRole("checkbox", { name: "Pause project work" }),
  ).toBeChecked();
  expect(
    await page.evaluate(
      () =>
        (window as typeof window & { __teamWrites: unknown[] }).__teamWrites
          .length,
    ),
  ).toBe(0);
  await team.getByRole("button", { name: "Save project permissions" }).click();
  await expect(team.getByText(/No unsaved changes/)).toBeVisible();
  const dialog = page.getByRole("dialog", {
    name: "Local project",
    exact: true,
  });
  await dialog.evaluate((element) => {
    element.scrollTop = 0;
  });
  await dialog.screenshot({
    path: testInfo.outputPath("project-team-permissions.png"),
  });
});

test("native inspection displays real scoped evidence without claiming agent readiness", async ({
  page,
}) => {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await team.getByRole("checkbox", { name: "Run project checks" }).check();
  await team.getByRole("checkbox", { name: "Pause project work" }).uncheck();
  const inspect = team.getByRole("button", {
    name: "Run isolated local inspection",
  });
  await expect(inspect).toBeDisabled();
  await team.getByRole("button", { name: "Save project permissions" }).click();
  await expect(inspect).toBeEnabled();
  await inspect.click();
  const result = team.getByRole("region", {
    name: "Local project inspection",
    exact: true,
  });
  await expect(
    result.getByText("/synthetic/project", { exact: true }),
  ).toBeVisible();
  await expect(result.getByText("synthetic-feature")).toBeVisible();
  await expect(result.getByText("Present — preserved")).toBeVisible();
  await expect(result.getByText(/does not start an AI agent/)).toBeVisible();
  const calls = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __inspectionCalls: Record<string, unknown>[];
        }
      ).__inspectionCalls,
  );
  expect(calls).toHaveLength(1);
  expect(calls[0]).toMatchObject({
    expectedBindingGeneration: 1,
    expectedTeamGeneration: 1,
    expectedOwner: "deadbeef".repeat(8),
    channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
  });
  await team.getByRole("checkbox", { name: "Edit project files" }).check();
  await expect(result.getByText("synthetic-feature")).toHaveCount(0);
  await expect(inspect).toBeDisabled();
});

test("foreign inspection receipts and sandbox errors never produce readiness evidence", async ({
  page,
}) => {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await team.getByRole("checkbox", { name: "Run project checks" }).check();
  await team.getByRole("checkbox", { name: "Pause project work" }).uncheck();
  await team.getByRole("button", { name: "Save project permissions" }).click();
  await page.evaluate(() => {
    (
      window as typeof window & { __inspectionWrongScope: boolean }
    ).__inspectionWrongScope = true;
  });
  const inspect = team.getByRole("button", {
    name: "Run isolated local inspection",
  });
  await inspect.click();
  const result = team.getByRole("region", {
    name: "Local project inspection",
    exact: true,
  });
  await expect(result.getByRole("alert")).toContainText(
    "Project or permissions changed",
  );
  await expect(result.getByText("synthetic-feature")).toHaveCount(0);
  await page.evaluate(() => {
    const bridge = window as typeof window & {
      __inspectionWrongScope: boolean;
      __inspectionFailure: boolean;
    };
    bridge.__inspectionWrongScope = false;
    bridge.__inspectionFailure = true;
  });
  await inspect.click();
  await expect(result.getByRole("alert")).toContainText(
    "Sandbox execution failed",
  );
  await expect(result.getByText("synthetic-feature")).toHaveCount(0);
});

async function openRecovery(page: Page, observedHash = "ab".repeat(32)) {
  await setup(page);
  await page.evaluate((hash) => {
    const bridge = window as typeof window & {
      __pendingNativeEdits: unknown[];
      __nativeObservedHash: string;
    };
    bridge.__nativeObservedHash = hash;
    bridge.__pendingNativeEdits = [
      {
        operation_id: "fixture-operation",
        channel_id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        relative_path: "existing.ts",
        before_sha256: "ab".repeat(32),
        after_sha256: "cd".repeat(32),
      },
    ];
  }, observedHash);
  await page.getByRole("tab", { name: "Edit recovery" }).click();
  const recovery = page.getByRole("region", {
    name: "Project edit recovery",
    exact: true,
  });
  await recovery
    .getByRole("button", { name: "Inspect and resolve this receipt" })
    .click();
  return recovery;
}

test("edit recovery requires stopped writer and independently observed hash", async ({
  page,
}) => {
  const recovery = await openRecovery(page);
  const record = recovery.getByRole("button", {
    name: "Record inspected outcome",
  });
  await expect(record).toBeDisabled();
  await recovery
    .getByLabel("Current file SHA-256, independently inspected")
    .fill("cd".repeat(32));
  await expect(record).toBeDisabled();
  await recovery
    .getByRole("checkbox", { name: /I independently confirmed/ })
    .check();
  await record.click();
  await expect(recovery.getByRole("alert")).toContainText("nothing reconciled");
  await recovery
    .getByLabel("Current file SHA-256, independently inspected")
    .fill("ab".repeat(32));
  await record.click();
  await expect(
    recovery.getByText("No unresolved native edit receipts."),
  ).toBeVisible();
  const writes = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __nativeReconciliationCalls: Record<string, unknown>[];
        }
      ).__nativeReconciliationCalls,
  );
  expect(writes).toHaveLength(1);
  expect(writes[0]).toMatchObject({
    operationId: "fixture-operation",
    expectedCurrentSha256: "ab".repeat(32),
    confirmedWriterStopped: true,
    expectedOwner: "deadbeef".repeat(8),
  });
});

test("missing-file recovery records absence without creating or replaying edits", async ({
  page,
}) => {
  const recovery = await openRecovery(page, "absent");
  await recovery
    .getByLabel("Current file SHA-256, independently inspected")
    .fill("absent");
  await recovery
    .getByRole("checkbox", { name: /I independently confirmed/ })
    .check();
  await recovery
    .getByRole("button", { name: "Record inspected outcome" })
    .click();
  await expect(
    recovery.getByText("No unresolved native edit receipts."),
  ).toBeVisible();
  const count = await page.evaluate(
    () =>
      (window as typeof window & { __teamWrites: unknown[] }).__teamWrites
        .length,
  );
  expect(count).toBe(0);
});

test("failed receipt refresh blocks resolving stale pending edits", async ({
  page,
}) => {
  const recovery = await openRecovery(page);
  await recovery
    .getByLabel("Current file SHA-256, independently inspected")
    .fill("ab".repeat(32));
  await recovery
    .getByRole("checkbox", { name: /I independently confirmed/ })
    .check();
  await page.evaluate(() => {
    (
      window as typeof window & { __nativeReceiptReadFailure: boolean }
    ).__nativeReceiptReadFailure = true;
  });
  await recovery.getByRole("button", { name: "Refresh edit receipts" }).click();
  await expect(recovery.getByRole("alert")).toContainText(
    "verification unavailable",
  );
  await expect(
    recovery.getByRole("button", { name: "Record inspected outcome" }),
  ).toBeDisabled();
});

test("a rebind cannot be authorized by an old permission draft", async ({
  page,
}) => {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await page.evaluate(() => {
    (
      window as typeof window & { __currentBindingGeneration: number }
    ).__currentBindingGeneration = 2;
  });
  await team.getByRole("button", { name: "Refresh team" }).click();
  await expect(team.getByText(/folder changed. Reload/)).toBeVisible();
  await expect(
    team.getByRole("button", { name: "Save project permissions" }),
  ).toBeDisabled();
  expect(
    await page.evaluate(
      () =>
        (window as typeof window & { __teamWrites: unknown[] }).__teamWrites
          .length,
    ),
  ).toBe(0);
});

test("a workspace refresh failure preserves the scoped team draft", async ({
  page,
}) => {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await team.getByRole("checkbox", { name: "Run project checks" }).check();
  await page.getByRole("tab", { name: "Workspace", exact: true }).click();
  await page.evaluate(() => {
    (
      window as typeof window & { __workspaceFailure: boolean }
    ).__workspaceFailure = true;
  });
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(
    page
      .getByRole("dialog", { name: "Local project", exact: true })
      .getByRole("status"),
  ).toHaveText("Binding unavailable");
  await page.evaluate(() => {
    (
      window as typeof window & { __workspaceFailure: boolean }
    ).__workspaceFailure = false;
  });
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await page.getByRole("tab", { name: "Team and permissions" }).click();
  await expect(team.getByLabel("Project coordinator")).toHaveValue(
    "ab".repeat(32),
  );
  await expect(
    team.getByRole("checkbox", { name: "Run project checks" }),
  ).toBeChecked();
  await expect(
    team.getByRole("button", { name: "Save project permissions" }),
  ).toBeEnabled();
});

test("failed team refresh blocks stale saves and permits recovery", async ({
  page,
}) => {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await page.evaluate(() => {
    (window as typeof window & { __teamFailure: boolean }).__teamFailure = true;
  });
  await team.getByRole("button", { name: "Refresh team" }).click();
  await expect(team.getByRole("alert")).toContainText(
    "Team verification unavailable",
  );
  await expect(
    team.getByRole("button", { name: "Save project permissions" }),
  ).toBeDisabled();
  await page.evaluate(() => {
    (window as typeof window & { __teamFailure: boolean }).__teamFailure =
      false;
  });
  await team.getByRole("button", { name: "Refresh team" }).click();
  await expect(
    team.getByRole("button", { name: "Save project permissions" }),
  ).toBeEnabled();
});

test("concurrent permission change preserves the draft but requires explicit discard", async ({
  page,
}) => {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await page.evaluate(() => {
    (window as typeof window & { __teamBump: () => void }).__teamBump();
  });
  await team.getByRole("button", { name: "Refresh team" }).click();
  await expect(team.getByRole("alert")).toContainText("permissions changed");
  await expect(
    team.getByRole("button", { name: "Save project permissions" }),
  ).toBeDisabled();
  await expect(team.getByLabel("Project coordinator")).toHaveValue(
    "ab".repeat(32),
  );
  await team.getByRole("button", { name: "Discard team changes" }).click();
  await expect(team.getByLabel("Project coordinator")).toHaveValue("");
});

test("lost save receipt reconciles without blind duplicate writes", async ({
  page,
}) => {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await page.evaluate(() => {
    (window as typeof window & { __teamLoseReply: boolean }).__teamLoseReply =
      true;
  });
  await team.getByRole("button", { name: "Save project permissions" }).click();
  await expect(team.getByRole("alert")).toContainText("Save outcome uncertain");
  await team.getByRole("button", { name: "Refresh team" }).click();
  await expect(team.getByRole("alert")).toContainText("permissions changed");
  await expect(
    team.getByRole("button", { name: "Save project permissions" }),
  ).toBeDisabled();
  expect(
    await page.evaluate(
      () =>
        (window as typeof window & { __teamWrites: unknown[] }).__teamWrites
          .length,
    ),
  ).toBe(1);
});

test("late save remains bound to the original channel", async ({ page }) => {
  const team = await setup(page);
  await team.getByLabel("Project coordinator").selectOption("ab".repeat(32));
  await page.evaluate(() => {
    (window as typeof window & { __teamDelay: boolean }).__teamDelay = true;
  });
  await team.getByRole("button", { name: "Save project permissions" }).click();
  await page.keyboard.press("Escape");
  await page.getByTestId("channel-random").click();
  await page
    .getByRole("button", { name: "Channel local project", exact: true })
    .click();
  await page.getByRole("tab", { name: "Team and permissions" }).click();
  const nextTeam = page.getByRole("region", {
    name: "Project team",
    exact: true,
  });
  await expect(nextTeam.getByLabel("Project coordinator")).toHaveValue("");
  await page.evaluate(() => {
    (window as typeof window & { __teamResolve: () => void }).__teamResolve();
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as typeof window & { __teamWrites: unknown[] }).__teamWrites
            .length,
      ),
    )
    .toBe(1);
  await expect(nextTeam.getByLabel("Project coordinator")).toHaveValue("");
  await expect(
    nextTeam.getByText(/Saved permission revision: 0/),
  ).toBeVisible();
});
