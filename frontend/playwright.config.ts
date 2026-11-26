import { defineConfig } from "@playwright/test";

const ci = Boolean(process.env.CI);
const mockPort = 8897;

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  workers: 1,
  fullyParallel: false,
  projects: [{ name: "agentstream", testMatch: /agentstream\.spec\.ts/ }],
  use: {
    baseURL: "http://127.0.0.1:8902",
    viewport: { width: 1440, height: 900 },
    ...(ci ? {} : { channel: "chrome" as const }),
  },
  webServer: [
    {
      command: `MOCK_PORT=${mockPort} node e2e/mock-server.mjs`,
      url: `http://127.0.0.1:${mockPort}/api/health`,
      reuseExistingServer: false,
      timeout: 30_000,
    },
    {
      command: `MM_UI_PORT=${mockPort} npx vite --host 127.0.0.1 --port 8902 --strictPort`,
      url: "http://127.0.0.1:8902",
      reuseExistingServer: false,
      timeout: 60_000,
    },
  ],
});
