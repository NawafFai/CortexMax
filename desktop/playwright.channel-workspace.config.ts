import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: [
    "channel-workspace.spec.ts",
    "project-channel-creation.spec.ts",
    "channel-project-team.spec.ts",
    "channel-project-board.spec.ts",
    "channel-project-review.spec.ts",
  ],
  workers: 1,
  retries: 0,
  timeout: 30_000,
  use: {
    ...devices["Desktop Chrome"],
    baseURL: "http://127.0.0.1:4289",
  },
  webServer: {
    command: "python3 -m http.server 4289 --bind 127.0.0.1 -d dist",
    url: "http://127.0.0.1:4289",
    reuseExistingServer: false,
  },
});
