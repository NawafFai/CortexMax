import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, openCreateChannelDialog } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

async function setup(page: Page) {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("app-sidebar").waitFor();
  await page.evaluate(() => {
    const bridge = window as typeof window & {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      __projectCalls: Record<string, unknown>[];
      __projectCreates: number;
      __loseProjectReply?: boolean;
      __folderConflict?: boolean;
    };
    const original = bridge.__TAURI_INTERNALS__.invoke.bind(
      bridge.__TAURI_INTERNALS__,
    );
    let pending: Record<string, unknown> | null = null;
    let receipt: unknown = null;
    bridge.__projectCalls = [];
    bridge.__projectCreates = 0;
    bridge.__TAURI_INTERNALS__.invoke = async (command, payload) => {
      if (command === "get_pending_project_channel") return pending;
      if (command === "abandon_project_channel_creation" && payload) {
        if (
          !pending ||
          pending.requestId !== payload.requestId ||
          payload.confirmedSafe !== true
        ) {
          throw new Error("explicit scoped reconciliation required");
        }
        pending = null;
        return null;
      }
      if (command === "pick_channel_workspace_folder")
        return "/synthetic/مشروع-existing";
      if (command === "create_channel_project_folder") {
        if (bridge.__folderConflict)
          throw new Error("Existing files are never replaced");
        return `${payload?.parent}/${payload?.name}`;
      }
      if (command === "create_project_channel" && payload) {
        bridge.__projectCalls.push(payload);
        const input = payload.input as Record<string, unknown>;
        if (!pending) {
          pending = input;
          bridge.__projectCreates += 1;
          receipt = await original("create_channel", input);
        } else if (pending.requestId !== input.requestId) {
          throw new Error("Resume saved creation, not a duplicate");
        }
        if (bridge.__loseProjectReply) {
          bridge.__loseProjectReply = false;
          throw new Error("Outcome uncertain; resume the saved request");
        }
        pending = null;
        return receipt;
      }
      return original(command, payload);
    };
  });
  await openCreateChannelDialog(page);
  return page.getByTestId("create-channel-dialog");
}

test("conversation-only creation keeps the original native route", async ({
  page,
}) => {
  const dialog = await setup(page);
  await dialog
    .getByTestId("create-channel-name")
    .fill("synthetic-conversation");
  await expect(dialog.getByLabel("Channel workspace")).toHaveValue(
    "conversation",
  );
  await dialog.getByTestId("create-channel-submit").click();
  await expect(dialog).not.toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as typeof window & { __projectCreates: number })
          .__projectCreates,
    ),
  ).toBe(0);
});

test("abandonment requires explicit inspection and never recreates the remote channel", async ({
  page,
}) => {
  const dialog = await setup(page);
  await page.evaluate(() => {
    (
      window as typeof window & { __loseProjectReply: boolean }
    ).__loseProjectReply = true;
  });
  await dialog.getByTestId("create-channel-name").fill("uncertain-project");
  await dialog.getByLabel("Channel workspace").selectOption("existing");
  await dialog
    .getByRole("button", { name: "Choose project folder", exact: true })
    .click();
  await dialog.getByTestId("create-channel-submit").click();
  const abandon = dialog.getByRole("button", { name: "Abandon local draft" });
  await expect(abandon).toBeVisible();
  await expect(abandon).toBeDisabled();
  await dialog.getByRole("checkbox", { name: /I inspected the relay/ }).check();
  await abandon.click();
  await expect(abandon).not.toBeVisible();
  await expect(dialog.getByLabel("Channel workspace")).toHaveValue(
    "conversation",
  );
  expect(
    await page.evaluate(
      () =>
        (window as typeof window & { __projectCreates: number })
          .__projectCreates,
    ),
  ).toBe(1);
});

test("late folder picker result cannot enter a reopened creation form", async ({
  page,
}) => {
  let dialog = await setup(page);
  await page.evaluate(() => {
    const bridge = window as typeof window & {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      __resolveOldFolder: (value: string) => void;
    };
    const original = bridge.__TAURI_INTERNALS__.invoke.bind(
      bridge.__TAURI_INTERNALS__,
    );
    bridge.__TAURI_INTERNALS__.invoke = (command, payload) =>
      command === "pick_channel_workspace_folder"
        ? new Promise((resolve) => {
            bridge.__resolveOldFolder = resolve;
          })
        : original(command, payload);
  });
  await dialog.getByLabel("Channel workspace").selectOption("existing");
  await dialog
    .getByRole("button", { name: "Choose project folder", exact: true })
    .click();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await openCreateChannelDialog(page);
  dialog = page.getByTestId("create-channel-dialog");
  await dialog.getByTestId("create-channel-name").fill("different-project");
  await dialog.getByLabel("Channel workspace").selectOption("existing");
  await page.evaluate(() => {
    (
      window as typeof window & { __resolveOldFolder: (value: string) => void }
    ).__resolveOldFolder("/synthetic/stale-project");
  });
  await waitForAnimations(page);
  await expect(dialog.locator("bdi")).toHaveCount(0);
  await expect(dialog.getByTestId("create-channel-submit")).toBeDisabled();
});

test("channel creation attaches a scoped existing folder without template agents", async ({
  page,
}, testInfo) => {
  const dialog = await setup(page);
  await dialog.getByTestId("create-channel-name").fill("synthetic-project");
  await dialog.getByLabel("Channel workspace").selectOption("existing");
  await expect(dialog.getByTestId("create-channel-submit")).toBeDisabled();
  await dialog
    .getByRole("button", { name: "Choose project folder", exact: true })
    .click();
  await expect(dialog.locator("bdi")).toHaveAttribute("dir", "ltr");
  await expect(dialog.getByTestId("create-channel-template")).toBeDisabled();
  await waitForAnimations(page);
  await dialog.screenshot({
    path: testInfo.outputPath("create-project-channel.png"),
  });
  await dialog.getByTestId("create-channel-submit").click();
  await expect(dialog).not.toBeVisible();
  const calls = await page.evaluate(
    () =>
      (window as typeof window & { __projectCalls: Record<string, unknown>[] })
        .__projectCalls,
  );
  expect(calls).toHaveLength(1);
  expect(calls[0].expectedOwner).toBe("deadbeef".repeat(8));
  expect(calls[0].expectedRelay).toBeTruthy();
  expect(calls[0].input).toMatchObject({
    name: "synthetic-project",
    root: "/synthetic/مشروع-existing",
    channelType: "stream",
  });
  expect(calls[0].input).not.toHaveProperty("templateId");
});

test("new folder conflict and lost channel reply preserve the original resumable request", async ({
  page,
}) => {
  let dialog = await setup(page);
  await page.evaluate(() => {
    const state = window as typeof window & {
      __folderConflict: boolean;
      __loseProjectReply: boolean;
    };
    state.__folderConflict = true;
    state.__loseProjectReply = true;
  });
  await dialog.getByTestId("create-channel-name").fill("recoverable-project");
  await dialog.getByLabel("Channel workspace").selectOption("new");
  await dialog.getByRole("button", { name: "Choose parent folder" }).click();
  await dialog.getByLabel("New project folder name").fill("new-child");
  await dialog.getByRole("button", { name: "Create empty folder" }).click();
  await expect(dialog.getByRole("alert")).toContainText("never replaced");
  await expect(dialog.getByTestId("create-channel-submit")).toBeDisabled();
  await page.evaluate(() => {
    (window as typeof window & { __folderConflict: boolean }).__folderConflict =
      false;
  });
  await dialog.getByRole("button", { name: "Create empty folder" }).click();
  await dialog.getByTestId("create-channel-submit").click();
  await expect(
    dialog.getByText("Outcome uncertain; resume the saved request"),
  ).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Resume saved creation" }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await openCreateChannelDialog(page);
  dialog = page.getByTestId("create-channel-dialog");
  await dialog.getByRole("button", { name: "Resume saved creation" }).click();
  await expect(dialog.getByTestId("create-channel-name")).toHaveValue(
    "recoverable-project",
  );
  await expect(dialog.getByLabel("Channel workspace")).toHaveValue("existing");
  await dialog.getByTestId("create-channel-submit").click();
  await expect(dialog).not.toBeVisible();
  const state = await page.evaluate(() => {
    const bridge = window as typeof window & {
      __projectCalls: Record<string, unknown>[];
      __projectCreates: number;
    };
    return { calls: bridge.__projectCalls, creates: bridge.__projectCreates };
  });
  expect(state.creates).toBe(1);
  expect(state.calls).toHaveLength(2);
  expect(state.calls[0]).toEqual(state.calls[1]);
});
