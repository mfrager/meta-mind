import { defineConfig } from "@playwright/test";

const ci = Boolean(process.env.CI);

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  workers: 1,
  fullyParallel: false,
  use: {
    baseURL: "http://127.0.0.1:8900",
    viewport: { width: 1440, height: 900 },
    ...(ci ? {} : { channel: "chrome" as const }),
  },
  webServer: [
    {
      command: "mkdir -p /tmp/mm-e2e-data && cargo run -q -p mm-serve -- --data-dir /tmp/mm-e2e-data --port 8899",
      cwd: "..",
      url: "http://127.0.0.1:8899/api/health",
      reuseExistingServer: false,
      timeout: 180_000,
    },
    {
      command: "npx vite --host 127.0.0.1 --port 8900 --strictPort",
      url: "http://127.0.0.1:8900",
      reuseExistingServer: false,
      timeout: 60_000,
    },
  ],
});
