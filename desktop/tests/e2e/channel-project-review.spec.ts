import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

async function setup(page: Page) {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.evaluate(() => {
    const bridge = window as typeof window & {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      __reviewCalls: { command: string; payload?: Record<string, unknown> }[];
      __handoffReleasedAt: number | null;
      __foreign: boolean;
      __teamReads: number;
    };
    const original = bridge.__TAURI_INTERNALS__.invoke.bind(
      bridge.__TAURI_INTERNALS__,
    );
    bridge.__reviewCalls = [];
    bridge.__handoffReleasedAt = null;
    bridge.__foreign = true;
    bridge.__teamReads = 0;
    const record = () => ({
      source: "file",
      label: "docs/HANDOFF.md",
      text: "# Handoff\n- [x] wired dispatcher\n- [ ] finish `src/parser.rs` tests\nEvidence: src/parser.rs",
      captured_at: 1789603200,
      binding_generation: 1,
      git_revision: "ab".repeat(20),
      dirty_at_capture: true,
      previous_writer_released_at: bridge.__handoffReleasedAt,
    });
    bridge.__TAURI_INTERNALS__.invoke = async (command, payload) => {
      if (/handoff|changes|previous_writer/.test(command))
        bridge.__reviewCalls.push({ command, payload });
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
          bridge.__teamReads += 1;
          return {
            generation: 2,
            binding_generation: 1,
            coordinator_pubkey: null,
            members: [],
            available_agents: [],
            paused: true,
            approved_checks: [],
            readiness: {
              state: "attached",
              checked_at: 1789603200,
              checks: [
                {
                  id: "handoff",
                  status: bridge.__handoffReleasedAt ? "ok" : "fail",
                  message: bridge.__handoffReleasedAt
                    ? "Handoff checkpoint reviewed; previous writer confirmed released."
                    : "A handoff checkpoint is recorded but the previous writer's release has not been confirmed.",
                  recovery:
                    "Review the handoff in the Review tab and confirm that the previous writer has stopped.",
                },
              ],
              blockers: bridge.__handoffReleasedAt
                ? []
                : [
                    "A handoff checkpoint is recorded but the previous writer's release has not been confirmed.",
                  ],
              git: {
                branch: "main",
                revision: "cd".repeat(20),
                dirty: true,
                verified_at: null,
              },
              last_inspection_at: null,
              pending_approvals: 0,
              running_tasks: 0,
            },
            execution_blockers: [],
          };
        case "inspect_channel_project_handoff":
          if (payload?.expectedBindingGeneration !== 1)
            throw new Error(
              "project binding changed; reload before inspecting",
            );
          return {
            channel_id: payload.channelId,
            binding_generation: 1,
            checked_at: 1789603400,
            record: record(),
            probed: ["HANDOFF.md", "docs/HANDOFF.md"],
            unfinished_items: ["finish `src/parser.rs` tests"],
            referenced_paths: ["src/parser.rs"],
            live: {
              branch: "main",
              revision: "cd".repeat(20),
              dirty: true,
              changed_paths: [{ status: " M", path: "src/parser.rs" }],
            },
            foreign_processes: bridge.__foreign
              ? [{ pid: 4242, command: "codex", cwd: "/synthetic/project" }]
              : [],
            foreign_scan_complete: true,
          };
        case "acknowledge_channel_project_previous_writer_release":
          if (payload?.confirmed !== true)
            throw new Error("explicit confirmation is required");
          bridge.__handoffReleasedAt = 1789603500;
          return record();
        case "import_channel_project_handoff_summary":
          return {
            source: "imported",
            label: payload?.label,
            text: payload?.text,
            captured_at: 1789603600,
            binding_generation: 1,
            git_revision: null,
            dirty_at_capture: null,
            previous_writer_released_at: null,
          };
        case "get_channel_project_changes":
          return {
            channel_id: payload?.channelId,
            binding_generation: 1,
            checked_at: 1789603700,
            changes: {
              actual_cwd: "/synthetic/project",
              repository: true,
              changed_paths: [
                { status: " M", path: "src/parser.rs" },
                { status: "??", path: "notes.txt" },
              ],
              paths_truncated: false,
              diff_stat: " src/parser.rs | 4 ++--\n 1 file changed",
              diff: "diff --git a/src/parser.rs b/src/parser.rs\n-old\n+new\n",
              diff_truncated: true,
              runtime: "native-read-only-checker-v1",
            },
            overlaps: [
              { path: "src/parser.rs", task_ids: ["task-1", "task-2"] },
            ],
            edit_receipts: [
              {
                operation_id: "op-1",
                relative_path: "src/parser.rs",
                before_sha256: "a".repeat(64),
                after_sha256: "b".repeat(64),
                state: "completed",
                created_at: 1789603100,
              },
            ],
            awaiting: ["task-1"],
          };
        default:
          return original(command, payload);
      }
    };
  });
  await page
    .getByRole("button", { name: "Channel local project", exact: true })
    .click();
  await page.getByRole("tab", { name: "Review" }).click();
  return page.getByRole("region", { name: "Project review" });
}

test("handoff inspection shows the checkpoint against live state and requires an explicit release before readiness clears", async ({
  page,
}) => {
  const review = await setup(page);
  await review.getByRole("button", { name: "Inspect handoff" }).click();
  const result = review.getByTestId("handoff-result");
  await expect(result.getByRole("status")).toContainText(
    "Handoff file: docs/HANDOFF.md",
  );
  await expect(
    result.getByText(/The checkpoint predates the current revision/),
  ).toBeVisible();
  await expect(
    result.getByRole("listitem").filter({ hasText: "finish `src/parser.rs`" }),
  ).toBeVisible();
  await expect(result.getByRole("alert")).toContainText("codex (4242)");
  const record = result.getByRole("button", {
    name: "Record previous writer release",
  });
  await expect(record).toBeDisabled();
  await result
    .getByRole("checkbox", {
      name: /I verified that the previous writer has stopped/,
    })
    .check();
  await record.click();
  await expect(result).toHaveCount(0);
  const calls = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __reviewCalls: {
            command: string;
            payload?: Record<string, unknown>;
          }[];
        }
      ).__reviewCalls,
  );
  expect(calls.map((call) => call.command)).toEqual([
    "inspect_channel_project_handoff",
    "acknowledge_channel_project_previous_writer_release",
  ]);
  expect(calls[1].payload).toMatchObject({ confirmed: true });
  await page.getByRole("tab", { name: "Team and permissions" }).click();
  await expect(
    page
      .getByRole("region", { name: "Project readiness" })
      .getByText(/previous writer confirmed released/),
  ).toBeVisible();
});

test("an imported summary is stored as a checkpoint without claiming live state", async ({
  page,
}) => {
  const review = await setup(page);
  await review
    .getByLabel("Label (for example the export file name)")
    .fill("chatgpt-export.md");
  await review
    .getByLabel("Summary text")
    .fill("انتهى العمل عند اختبار المحلل\nsrc/parser.rs");
  await review.getByRole("button", { name: "Store imported summary" }).click();
  await expect(review.getByLabel("Summary text")).toHaveValue("");
  const calls = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __reviewCalls: {
            command: string;
            payload?: Record<string, unknown>;
          }[];
        }
      ).__reviewCalls,
  );
  expect(calls[0]).toMatchObject({
    command: "import_channel_project_handoff_summary",
    payload: { label: "chatgpt-export.md", expectedBindingGeneration: 1 },
  });
});

test("change review shows changed paths, truncation, overlaps and receipts without any merge action", async ({
  page,
}, testInfo) => {
  const review = await setup(page);
  await review.getByRole("button", { name: "Review changes" }).click();
  const result = review.getByTestId("changes-result");
  await page.screenshot({
    fullPage: true,
    path: testInfo.outputPath("review-changes.png"),
  });
  await expect(result.getByRole("status")).toContainText(
    "2 changed path(s); 1 task(s) awaiting review or acceptance.",
  );
  await expect(result.getByRole("alert")).toContainText(
    "src/parser.rs (task-1, task-2)",
  );
  await expect(
    result.getByRole("list", { name: "Changed paths" }).getByRole("listitem"),
  ).toHaveCount(2);
  await expect(result.getByText("Unified diff (truncated)")).toBeVisible();
  await expect(result.getByText("Native edit receipts (1)")).toBeVisible();
  await expect(
    result.getByText(/Merged and deployed: not performed by Cortex Max/),
  ).toBeVisible();
  await expect(
    review.getByRole("button", { name: /merge|commit|deploy|push/i }),
  ).toHaveCount(0);
});
