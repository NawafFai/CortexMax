import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

async function openWorkspace(page: Page) {
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
      __workspaceFailure?: boolean;
      __workspaceWrites?: Record<string, unknown>[];
      __workspacePending?: { id: string; agent_pubkey: string }[];
      __workspaceReleases?: Record<string, unknown>[];
    };
    const original = bridge.__TAURI_INTERNALS__.invoke.bind(
      bridge.__TAURI_INTERNALS__,
    );
    let binding: {
      channel_id: string;
      generation: number;
      root: string | null;
    } | null = null;
    bridge.__workspaceWrites = [];
    bridge.__workspaceReleases = [];
    bridge.__TAURI_INTERNALS__.invoke = async (command, payload) => {
      if (command === "pick_channel_workspace_folder")
        return "/synthetic/مشروع-alpha";
      if (command === "get_channel_workspace") {
        if (bridge.__workspaceFailure)
          throw new Error("folder verification unavailable");
        return {
          pending_provider_attempts: bridge.__workspacePending ?? [],
          checked_at: 1789603200,
          binding,
          folder_readable: Boolean(binding?.root),
          folder_error: null,
          execution_ready: false,
          execution_blocker: "unsupported enforcement",
        };
      }
      if (command === "set_channel_workspace" && payload) {
        if (payload.expectedGeneration !== (binding?.generation ?? 0))
          throw new Error("stale binding");
        bridge.__workspaceWrites?.push(payload);
        binding = {
          channel_id: String(payload.channelId),
          generation: (binding?.generation ?? 0) + 1,
          root: payload.root as string | null,
        };
        return binding;
      }
      if (command === "confirm_channel_workspace_writer_release" && payload) {
        if (
          payload.writerReleased !== true ||
          !bridge.__workspacePending?.some(
            (attempt) => attempt.id === payload.attemptId,
          )
        ) {
          throw new Error(
            "release confirmation does not match pending attempt",
          );
        }
        bridge.__workspaceReleases?.push(payload);
        bridge.__workspacePending = [];
        return null;
      }
      return original(command, payload);
    };
  });
  const trigger = page.getByRole("button", {
    name: "Channel local project",
    exact: true,
  });
  await trigger.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", {
    name: "Local project",
    exact: true,
  });
  await expect(dialog).toBeVisible();
  return dialog;
}

test("local binding uses scoped IPC and never advertises agent readiness", async ({
  page,
}, testInfo) => {
  const dialog = await openWorkspace(page);
  await expect(dialog.getByRole("status")).toHaveText("Conversation only");
  await dialog.getByRole("button", { name: "Choose existing folder" }).click();
  await dialog.getByRole("button", { name: "Attach selected folder" }).click();
  await expect(dialog.getByRole("status")).toHaveText("Project attached");
  await expect(
    dialog.getByText(
      "Folder readable by this desktop — agent access not verified.",
    ),
  ).toBeVisible();
  await expect(dialog.getByText(/Execution is not ready/)).toBeVisible();
  await expect(
    dialog.getByText("/synthetic/مشروع-alpha", { exact: true }),
  ).toHaveAttribute("dir", "ltr");
  await waitForAnimations(page);
  await dialog.screenshot({
    path: testInfo.outputPath("channel-workspace-attached.png"),
  });
  await dialog.getByRole("button", { name: "Detach project" }).click();
  await expect(dialog.getByRole("status")).toHaveText("Conversation only");
  const writes = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __workspaceWrites: Record<string, unknown>[];
        }
      ).__workspaceWrites,
  );
  expect(writes).toHaveLength(2);
  expect(writes[0].expectedOwner).toBe("deadbeef".repeat(8));
  expect(writes[0].expectedRelay).toBeTruthy();
  expect(writes[0].channelId).toBe("9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50");
  expect(writes.map((write) => write.expectedGeneration)).toEqual([0, 1]);
  expect(writes[1].root).toBeNull();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "Channel local project", exact: true }),
  ).toBeFocused();
});

test("failed refresh disables stale saves and leaves recovery available", async ({
  page,
}) => {
  const dialog = await openWorkspace(page);
  await dialog.getByRole("button", { name: "Choose existing folder" }).click();
  await page.evaluate(() => {
    (
      window as typeof window & { __workspaceFailure: boolean }
    ).__workspaceFailure = true;
  });
  await dialog.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(dialog.getByRole("status")).toHaveText("Binding unavailable");
  await expect(dialog.getByRole("alert")).toContainText(
    "folder verification unavailable",
  );
  await expect(
    dialog.getByRole("button", { name: "Attach selected folder" }),
  ).toBeDisabled();
  await expect(
    dialog.getByRole("button", { name: "Refresh", exact: true }),
  ).toBeEnabled();
  await page.evaluate(() => {
    (
      window as typeof window & { __workspaceFailure: boolean }
    ).__workspaceFailure = false;
  });
  await dialog.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(
    dialog.getByRole("button", { name: "Attach selected folder" }),
  ).toBeEnabled();
});

test("a late folder selection cannot move into a different channel", async ({
  page,
}) => {
  const dialog = await openWorkspace(page);
  await page.evaluate(() => {
    const bridge = window as typeof window & {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      __resolveWorkspacePicker?: (path: string) => void;
      __workspacePickerFinished?: boolean;
    };
    const original = bridge.__TAURI_INTERNALS__.invoke.bind(
      bridge.__TAURI_INTERNALS__,
    );
    bridge.__TAURI_INTERNALS__.invoke = async (command, payload) => {
      if (command === "pick_channel_workspace_folder") {
        return new Promise<string>((resolve) => {
          bridge.__resolveWorkspacePicker = resolve;
        }).then((path) => {
          bridge.__workspacePickerFinished = true;
          return path;
        });
      }
      return original(command, payload);
    };
  });
  await dialog.getByRole("button", { name: "Choose existing folder" }).click();
  await page.keyboard.press("Escape");
  await page.getByTestId("channel-random").click();
  await page
    .getByRole("button", { name: "Channel local project", exact: true })
    .click();
  const nextDialog = page.getByRole("dialog", {
    name: "Local project",
    exact: true,
  });
  await expect(nextDialog.getByRole("status")).toHaveText("Conversation only");
  await page.evaluate(() => {
    (
      window as typeof window & {
        __resolveWorkspacePicker: (path: string) => void;
      }
    ).__resolveWorkspacePicker("/synthetic/old-channel");
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as typeof window & { __workspacePickerFinished?: boolean })
            .__workspacePickerFinished,
      ),
    )
    .toBe(true);
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
  );
  await expect(
    nextDialog.getByRole("button", { name: "Attach selected folder" }),
  ).toBeDisabled();
  await expect(nextDialog.getByText("/synthetic/old-channel")).toHaveCount(0);
});

test("uncertain remote execution requires explicit owner release before saving", async ({
  page,
}) => {
  const dialog = await openWorkspace(page);
  await page.evaluate(() => {
    (
      window as typeof window & {
        __workspacePending: { id: string; agent_pubkey: string }[];
      }
    ).__workspacePending = [
      { id: "synthetic-attempt", agent_pubkey: "ab".repeat(32) },
    ];
  });
  await dialog.getByRole("button", { name: "Refresh", exact: true }).click();
  await dialog.getByRole("button", { name: "Choose existing folder" }).click();
  await expect(
    dialog.getByRole("button", { name: "Attach selected folder" }),
  ).toBeDisabled();
  const release = dialog.getByRole("button", { name: "Record writer release" });
  await expect(release).toBeDisabled();
  await dialog
    .getByRole("checkbox", {
      name: "I independently verified this remote writer has stopped.",
    })
    .check();
  await release.click();
  await expect(
    dialog.getByRole("button", { name: "Attach selected folder" }),
  ).toBeEnabled();
  const confirmations = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __workspaceReleases: Record<string, unknown>[];
        }
      ).__workspaceReleases,
  );
  expect(confirmations).toHaveLength(1);
  expect(confirmations[0]).toMatchObject({
    attemptId: "synthetic-attempt",
    writerReleased: true,
    expectedOwner: "deadbeef".repeat(8),
  });
  await expect(dialog.getByText(/Execution is not ready/)).toBeVisible();
});
