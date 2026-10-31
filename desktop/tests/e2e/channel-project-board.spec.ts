import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

type Task = {
  task_id: string;
  channel_id: string;
  title: string;
  description: string;
  state: string;
  state_before_pause: string | null;
  assignee_pubkey: string | null;
  created_by: string;
  required_inputs: string[];
  dependencies: string[];
  outputs: { kind: string; reference: string; sha256?: string }[];
  return_to: string;
  blocker: string | null;
  review: {
    reviewer_pubkey: string;
    verdict: string;
    notes: string;
    reviewed_at: number;
  } | null;
  report: Record<string, string | number> | null;
  revision: number;
  binding_generation: number;
  team_generation: number;
  created_at: number;
  updated_at: number;
  dispatch_state: string;
  dispatch_target: string | null;
  dispatch_event_id: string | null;
  dispatch_attempted_at: number | null;
};

async function setup(page: Page, readinessState = "execution_authorized") {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.evaluate((initialState) => {
    const bridge = window as typeof window & {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      __boardCalls: { command: string; payload?: Record<string, unknown> }[];
      __readinessState: string;
      __tasks: Task[];
      __approvals: Record<string, unknown>[];
      __teamGeneration: number;
      __approvedChecks: string[];
      __paused: boolean;
      __staleTransition?: boolean;
    };
    const original = bridge.__TAURI_INTERNALS__.invoke.bind(
      bridge.__TAURI_INTERNALS__,
    );
    const coordinator = "ab".repeat(32);
    const writer = "cd".repeat(32);
    const reviewer = "ef".repeat(32);
    bridge.__boardCalls = [];
    bridge.__readinessState = initialState;
    bridge.__tasks = [];
    bridge.__teamGeneration = 3;
    bridge.__approvedChecks = ["cargo test --offline"];
    bridge.__paused = false;
    bridge.__approvals = [
      {
        request_id: "req-1",
        channel_id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        agent_pubkey: writer,
        category: "execute",
        summary: "pnpm test",
        reason: "not in the owner-approved check list",
        state: "pending",
        requested_at: 1789603200,
        decided_at: null,
        decision_note: null,
        binding_generation: 1,
        team_generation: 3,
      },
      {
        request_id: "req-2",
        channel_id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        agent_pubkey: writer,
        category: "publish",
        summary: "git push origin main",
        reason: "publishing always needs a per-action owner decision",
        state: "pending",
        requested_at: 1789603300,
        decided_at: null,
        decision_note: null,
        binding_generation: 1,
        team_generation: 3,
      },
    ];
    const team = () => ({
      generation: bridge.__teamGeneration,
      binding_generation: 1,
      coordinator_pubkey: coordinator,
      paused: bridge.__paused,
      approved_checks: bridge.__approvedChecks,
      roles: [
        "coordinator",
        "manager",
        "writer",
        "tester",
        "reviewer",
        "member",
      ],
      capabilities: [
        "read",
        "edit",
        "run_checks",
        "manage_team",
        "assign_tasks",
        "review",
        "create_private_channels",
        "commit_merge",
      ],
      members: [
        {
          pubkey: coordinator,
          display_name: "CORTEX",
          role: "coordinator",
          capabilities: ["read", "assign_tasks", "manage_team"],
        },
        {
          pubkey: writer,
          display_name: "Byte",
          role: "writer",
          capabilities: ["read", "edit", "run_checks"],
        },
        {
          pubkey: reviewer,
          display_name: "Sentinel",
          role: "reviewer",
          capabilities: ["read", "review"],
        },
      ],
      available_agents: [
        {
          pubkey: coordinator,
          display_name: "CORTEX",
          runtime_id: "claude",
          local: true,
          enforcement: {
            kind: "gate",
            profile: {
              harness: "claude",
              gate_mode: "default",
              write_confinement: "harness_gate",
              read_confinement: "unconfined",
              command_confinement: "harness_gate",
              network_blocked: false,
              evidence: "synthetic",
            },
          },
        },
        {
          pubkey: writer,
          display_name: "Byte",
          runtime_id: "codex",
          local: true,
          enforcement: {
            kind: "gate",
            profile: {
              harness: "codex",
              gate_mode: "read-only",
              write_confinement: "os_sandbox",
              read_confinement: "unconfined",
              command_confinement: "unconfined",
              network_blocked: true,
              evidence: "synthetic",
            },
          },
        },
        {
          pubkey: reviewer,
          display_name: "Sentinel",
          runtime_id: "goose",
          local: true,
          enforcement: {
            kind: "unverified",
            reason: "Goose sends no permission requests",
          },
        },
      ],
      readiness: {
        state: bridge.__readinessState,
        checked_at: 1789603200,
        checks: [
          {
            id: "folder",
            status: "ok",
            message: "Project folder exists and is readable by this desktop.",
          },
          {
            id: `member:${reviewer}:enforcement`,
            status: bridge.__readinessState === "attached" ? "fail" : "ok",
            message:
              bridge.__readinessState === "attached"
                ? "Sentinel runs on `goose` which has no verified project enforcement"
                : "Sentinel: harness gate.",
            recovery:
              "Release this member or switch it to Claude Code, Codex, or Buzz Agent.",
          },
        ],
        blockers:
          bridge.__readinessState === "attached"
            ? [
                "Sentinel runs on `goose` which has no verified project enforcement",
              ]
            : [],
        git: {
          branch: "synthetic-feature",
          revision: "ab".repeat(20),
          dirty: true,
          verified_at: 1789603200,
        },
        last_inspection_at: 1789603200,
        pending_approvals: bridge.__approvals.filter(
          (request) => request.state === "pending",
        ).length,
        running_tasks: bridge.__tasks.filter((task) => task.state === "running")
          .length,
      },
      execution_blockers:
        bridge.__readinessState === "attached"
          ? [
              "Sentinel runs on `goose` which has no verified project enforcement",
            ]
          : [],
    });
    bridge.__TAURI_INTERNALS__.invoke = async (command, payload) => {
      if (
        /project|approval|dispatch/.test(command) &&
        command !== "get_channel_workspace"
      )
        bridge.__boardCalls.push({ command, payload });
      switch (command) {
        case "get_channel_workspace":
          return {
            pending_provider_attempts: [],
            checked_at: 1789603200,
            binding: {
              channel_id: payload?.channelId,
              generation: 1,
              root: "/synthetic/project",
            },
            folder_readable: true,
            folder_error: null,
            execution_ready: false,
            execution_blocker: "",
          };
        case "get_channel_project_team":
          return team();
        case "set_channel_project_paused":
          if (payload?.expectedGeneration !== bridge.__teamGeneration)
            throw new Error("project team changed; reload before saving");
          bridge.__teamGeneration += 1;
          bridge.__paused = Boolean(payload?.paused);
          return team();
        case "list_project_approval_requests":
          return bridge.__approvals;
        case "decide_project_approval_request": {
          const request = bridge.__approvals.find(
            (item) => item.request_id === payload?.requestId,
          );
          if (request?.state !== "pending")
            throw new Error("approval request is not pending");
          request.state = payload?.approve ? "approved" : "denied";
          request.decided_at = 1789603400;
          if (payload?.approve && payload?.addToApprovedChecks) {
            if (payload.expectedTeamGeneration !== bridge.__teamGeneration)
              throw new Error("project team changed; reload before saving");
            bridge.__approvedChecks = [
              ...bridge.__approvedChecks,
              request.summary as string,
            ];
            bridge.__teamGeneration += 1;
          }
          return request;
        }
        case "list_channel_project_tasks":
          return bridge.__tasks;
        case "create_channel_project_task": {
          const input = payload?.input as Record<string, unknown>;
          const task: Task = {
            task_id: `task-${bridge.__tasks.length + 1}`,
            channel_id: String(payload?.channelId),
            title: String(input.title),
            description: String(input.description),
            state: input.assigneePubkey ? "assigned" : "planned",
            state_before_pause: null,
            assignee_pubkey: (input.assigneePubkey as string | null) ?? null,
            created_by: "owner",
            required_inputs: [],
            dependencies: [],
            outputs: [],
            return_to: "owner",
            blocker: null,
            review: null,
            report: null,
            revision: input.assigneePubkey ? 2 : 1,
            binding_generation: 1,
            team_generation: bridge.__teamGeneration,
            created_at: 1789603200,
            updated_at: 1789603200,
            dispatch_state: input.assigneePubkey ? "pending" : "none",
            dispatch_target: (input.assigneePubkey as string | null) ?? null,
            dispatch_event_id: null,
            dispatch_attempted_at: null,
          };
          bridge.__tasks = [task, ...bridge.__tasks];
          return task;
        }
        case "dispatch_channel_project_tasks":
          for (const task of bridge.__tasks)
            if (task.dispatch_state === "pending") {
              task.dispatch_state = "sent";
              task.dispatch_event_id = "evt-1";
            }
          return {
            published: 1,
            in_app: 0,
            reconciled: 0,
            uncertain: 0,
            deferred: 0,
          };
        case "transition_channel_project_task": {
          const task = bridge.__tasks.find(
            (item) => item.task_id === payload?.taskId,
          );
          if (!task) throw new Error("unknown task");
          if (
            bridge.__staleTransition ||
            task.revision !== payload?.expectedRevision
          )
            throw new Error("task changed; reload the board before acting");
          const action = payload?.action as { action: string };
          if (action.action === "accept") {
            if (task.state !== "awaiting_owner_approval")
              throw new Error(
                "cannot accept a task without an approved review",
              );
            task.state = "completed";
          } else if (action.action === "pause") {
            task.state_before_pause = task.state;
            task.state = "paused";
          } else if (action.action === "resume") {
            task.state = task.state_before_pause ?? "planned";
          } else if (action.action === "cancel") {
            task.state = "cancelled";
          }
          task.revision += 1;
          return task;
        }
        case "resolve_channel_project_task_dispatch": {
          const task = bridge.__tasks.find(
            (item) => item.task_id === payload?.taskId,
          );
          if (task?.dispatch_state !== "uncertain")
            throw new Error("dispatch is not uncertain at this revision");
          task.dispatch_state = payload?.observedEventId ? "sent" : "pending";
          task.dispatch_event_id = (payload?.observedEventId as string) ?? null;
          return task;
        }
        default:
          return original(command, payload);
      }
    };
  }, readinessState);
  await page
    .getByRole("button", { name: "Channel local project", exact: true })
    .click();
}

function calls(page: Page) {
  return page.evaluate(
    () =>
      (
        window as typeof window & {
          __boardCalls: {
            command: string;
            payload?: Record<string, unknown>;
          }[];
        }
      ).__boardCalls,
  );
}

test("readiness ladder gates assignment and names the failing member with a recovery step", async ({
  page,
}, testInfo) => {
  await setup(page, "attached");
  await expect(page.getByTestId("workspace-readiness-state")).toHaveText(
    "attached",
  );
  await page.getByRole("tab", { name: "Team and permissions" }).click();
  const readiness = page.getByRole("region", { name: "Project readiness" });
  await expect(readiness.getByTestId("project-readiness-state")).toHaveText(
    "Project attached — runtime not configured",
  );
  await expect(
    readiness.getByText(/Next step: Release this member or switch it/),
  ).toBeVisible();
  await expect(
    page.getByRole("group", { name: "Sentinel", exact: true }),
  ).toContainText("No verified enforcement");
  await page.getByRole("tab", { name: "Tasks" }).click();
  const board = page.getByRole("region", { name: "Project task board" });
  await board.getByLabel("Title").fill("Fix login");
  await board
    .getByLabel("What exactly should be delivered")
    .fill("إصلاح خطأ تسجيل الدخول مع اختبار");
  await board.getByRole("button", { name: "Plan and assign task" }).click();
  await expect(board.getByRole("alert")).toContainText(
    "Execution is not authorized",
  );
  expect(
    (await calls(page)).filter(
      (call) => call.command === "create_channel_project_task",
    ),
  ).toHaveLength(0);
  await board.getByLabel("Assign to").selectOption("");
  await board.getByRole("button", { name: "Plan task" }).click();
  await expect(
    board
      .getByRole("region", { name: "Queued" })
      .getByText("Fix login — planned", {
        exact: false,
      }),
  ).toBeVisible();
  await expect(
    board.getByRole("region", { name: "Queued" }).getByRole("button", {
      name: "Assign",
    }),
  ).toBeDisabled();
  await page.getByRole("tab", { name: "Team and permissions" }).click();
  await page.screenshot({
    fullPage: true,
    path: testInfo.outputPath("readiness-attached.png"),
  });
});

test("owner plans and assigns a bounded task from the keyboard, then sees the dispatch receipt", async ({
  page,
}) => {
  await setup(page);
  await page.getByRole("tab", { name: "Tasks" }).click();
  const board = page.getByRole("region", { name: "Project task board" });
  await expect(board.getByRole("status")).toHaveText("Execution authorized.");
  await board.getByLabel("Title").focus();
  await page.keyboard.type("Add retry");
  await page.keyboard.press("Tab");
  await page.keyboard.type("أضف إعادة محاولة محدودة عند فشل الشبكة");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  await expect(
    board.getByRole("button", { name: "Plan and assign task" }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  const active = board.getByRole("region", { name: "In progress" });
  await expect(
    active.getByText(/Add retry — assigned \(revision 2\)/),
  ).toBeVisible();
  await expect(
    active.getByText(/Notification delivered \(event evt-1\)/),
  ).toBeVisible();
  const recorded = await calls(page);
  const create = recorded.find(
    (call) => call.command === "create_channel_project_task",
  );
  expect(create?.payload).toMatchObject({
    channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
    input: {
      title: "Add retry",
      assigneePubkey: "ab".repeat(32),
      returnTo: "owner",
    },
  });
  expect(
    recorded.some((call) => call.command === "dispatch_channel_project_tasks"),
  ).toBe(true);
  // Acceptance is never offered before an approved independent review.
  await expect(
    active.getByRole("button", { name: "Accept delivery" }),
  ).toHaveCount(0);
});

test("acceptance appears only after an approved review, the final report renders mixed-direction, and stale revisions are refused", async ({
  page,
}, testInfo) => {
  await setup(page);
  await page.evaluate(() => {
    const bridge = window as typeof window & { __tasks: Task[] };
    bridge.__tasks = [
      {
        task_id: "task-9",
        channel_id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        title: "Harden parser",
        description: "تشديد المحلل",
        state: "awaiting_owner_approval",
        state_before_pause: null,
        assignee_pubkey: "cd".repeat(32),
        created_by: "ab".repeat(32),
        required_inputs: [],
        dependencies: [],
        outputs: [
          { kind: "file", reference: "src/parser.rs", sha256: "a".repeat(64) },
        ],
        return_to: "ab".repeat(32),
        blocker: null,
        review: {
          reviewer_pubkey: "ef".repeat(32),
          verdict: "approved",
          notes: "Bounds checked; tests added.",
          reviewed_at: 1789603300,
        },
        report: {
          requested: "تشديد محلل الإدخال ضد القيم الطويلة",
          delivered: "أُضيفت حدود الطول في\nsrc/parser.rs\nمع اختبار وحدة",
          verified: "نجح الفحص المعتمد\ncargo test --offline",
          failed_or_unverified: "لم يُختبر على Windows",
          decision_needed: "اعتماد التسليم",
          author_pubkey: "ab".repeat(32),
          filed_at: 1789603350,
        },
        revision: 7,
        binding_generation: 1,
        team_generation: 3,
        created_at: 1789603200,
        updated_at: 1789603350,
        dispatch_state: "sent",
        dispatch_target: "ab".repeat(32),
        dispatch_event_id: "evt-7",
        dispatch_attempted_at: 1789603351,
      },
    ];
  });
  await page.getByRole("tab", { name: "Tasks" }).click();
  const board = page.getByRole("region", { name: "Project task board" });
  const card = board.getByRole("region", { name: "Delivered / review" });
  await expect(
    card.getByText(/Independent review by Sentinel: approved/),
  ).toBeVisible();
  const report = card.getByRole("region", { name: "Final report" });
  await expect(report).toHaveAttribute("dir", "rtl");
  await expect(
    report.getByText("src/parser.rs", { exact: true }),
  ).toHaveAttribute("dir", "ltr");
  await expect(
    report.getByText("cargo test --offline", { exact: true }),
  ).toHaveAttribute("dir", "ltr");
  await expect(
    report.getByText("اعتماد التسليم", { exact: true }),
  ).toHaveAttribute("dir", "rtl");
  await page.screenshot({
    fullPage: true,
    path: testInfo.outputPath("task-final-report.png"),
  });
  await page.evaluate(() => {
    (
      window as typeof window & { __staleTransition?: boolean }
    ).__staleTransition = true;
  });
  await card.getByRole("button", { name: "Accept delivery" }).click();
  await expect(board.getByRole("alert")).toContainText("task changed");
  await expect(card.getByText(/awaiting_owner_approval/)).toBeVisible();
  await page.evaluate(() => {
    (
      window as typeof window & { __staleTransition?: boolean }
    ).__staleTransition = false;
  });
  await card.getByRole("button", { name: "Accept delivery" }).click();
  await expect(
    board
      .getByRole("region", { name: "Completed / cancelled" })
      .getByText(/Harden parser — completed/),
  ).toBeVisible();
});

test("uncertain notifications are reconciled by the owner, never resent silently", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(() => {
    const bridge = window as typeof window & { __tasks: Task[] };
    bridge.__tasks = [
      {
        task_id: "task-5",
        channel_id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        title: "Write docs",
        description: "توثيق",
        state: "assigned",
        state_before_pause: null,
        assignee_pubkey: "cd".repeat(32),
        created_by: "owner",
        required_inputs: [],
        dependencies: [],
        outputs: [],
        return_to: "owner",
        blocker: null,
        review: null,
        report: null,
        revision: 2,
        binding_generation: 1,
        team_generation: 3,
        created_at: 1789603200,
        updated_at: 1789603200,
        dispatch_state: "uncertain",
        dispatch_target: "cd".repeat(32),
        dispatch_event_id: null,
        dispatch_attempted_at: 1789603201,
      },
    ];
  });
  await page.getByRole("tab", { name: "Tasks" }).click();
  const board = page.getByRole("region", { name: "Project task board" });
  await expect(board.getByText(/Notification outcome unknown/)).toBeVisible();
  const arrived = board.getByRole("button", { name: "It arrived" });
  await expect(arrived).toBeDisabled();
  await board.getByLabel("Observed notification event id").fill("evt-seen");
  await arrived.click();
  await expect(
    board.getByText(/Notification delivered \(event evt-seen\)/),
  ).toBeVisible();
  const resolve = (await calls(page)).find(
    (call) => call.command === "resolve_channel_project_task_dispatch",
  );
  expect(resolve?.payload).toMatchObject({
    taskId: "task-5",
    expectedRevision: 2,
    observedEventId: "evt-seen",
  });
});

test("approving a refused command adds it to the approved checks with the team generation fence", async ({
  page,
}, testInfo) => {
  await setup(page);
  await page.getByRole("tab", { name: /Approvals/ }).click();
  const approvals = page.getByRole("region", { name: "Project approvals" });
  await page.screenshot({
    fullPage: true,
    path: testInfo.outputPath("approvals-pending.png"),
  });
  const pending = approvals.getByRole("list", {
    name: "Pending approval requests",
  });
  await expect(pending.getByRole("listitem")).toHaveCount(2);
  const command = pending
    .getByRole("listitem")
    .filter({ hasText: "pnpm test" });
  await command
    .getByRole("checkbox", { name: /Add this exact command line/ })
    .check();
  await command.getByLabel("Decision note (optional)").fill("ok for CI");
  await command.getByRole("button", { name: "Approve" }).click();
  await expect(pending.getByRole("listitem")).toHaveCount(1);
  const decision = (await calls(page)).find(
    (call) => call.command === "decide_project_approval_request",
  );
  expect(decision?.payload).toMatchObject({
    requestId: "req-1",
    approve: true,
    addToApprovedChecks: true,
    expectedTeamGeneration: 3,
    note: "ok for CI",
  });
  // The publish request has no "add to checks" option: it stays per action.
  const publish = pending.getByRole("listitem").filter({ hasText: "git push" });
  await expect(publish.getByRole("checkbox")).toHaveCount(0);
  await publish.getByRole("button", { name: "Deny" }).click();
  await expect(approvals.getByText("No pending requests.")).toBeVisible();
  await page.getByRole("tab", { name: "Team and permissions" }).click();
  const checks = page.getByRole("region", { name: "Approved checks" });
  await expect(checks.getByText("pnpm test", { exact: true })).toBeVisible();
});

test("pause and resume use the saved generation without stopping runtimes", async ({
  page,
}) => {
  await setup(page);
  await page.getByRole("tab", { name: "Team and permissions" }).click();
  const team = page.getByRole("region", { name: "Project team", exact: true });
  await expect(team.getByText("Dispatch is active.")).toBeVisible();
  await team.getByRole("button", { name: "Pause project work" }).click();
  await expect(team.getByText("Dispatch is paused.")).toBeVisible();
  await team.getByRole("button", { name: "Resume project work" }).click();
  await expect(team.getByText("Dispatch is active.")).toBeVisible();
  const pauses = (await calls(page)).filter(
    (call) => call.command === "set_channel_project_paused",
  );
  expect(pauses.map((call) => call.payload)).toEqual([
    expect.objectContaining({ expectedGeneration: 3, paused: true }),
    expect.objectContaining({ expectedGeneration: 4, paused: false }),
  ]);
});
