import { defineConfig } from "@playwright/test";

const ci = Boolean(process.env.CI);
const tmp = process.env.TMPDIR || "/tmp";
const build =
  "flock /home/mfrager/tmp/mm-e2e-build.lock env TMPDIR=/home/mfrager/tmp CARGO_TARGET_DIR=/home/mfrager/Build/metamind/target cargo build -q -p mm-serve";
const bin = "/home/mfrager/Build/metamind/target/debug/mm-serve";

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  workers: 1,
  fullyParallel: false,
  projects: [
    { name: "kernel", testMatch: /console\.spec\.ts/ },
    { name: "scripted", testMatch: /controller\.spec\.ts/ },
  ],
  use: {
    baseURL: "http://127.0.0.1:8902",
    viewport: { width: 1440, height: 900 },
    ...(ci ? {} : { channel: "chrome" as const }),
    launchOptions: {
      args: ["--disable-dev-shm-usage", `--disk-cache-dir=${tmp}/mm-chrome-cache`],
    },
  },
  webServer: [
    {
      command: `mkdir -p /home/mfrager/tmp/mm-e2e-data && ${build} && OPENAI_BASE_URL= OPENAI_API_KEY= ${bin} --data-dir /home/mfrager/tmp/mm-e2e-data --port 8896`,
      cwd: "..",
      url: "http://127.0.0.1:8896/api/health",
      reuseExistingServer: false,
      timeout: 180_000,
    },
    {
      command: "MM_UI_PORT=8896 npx vite --host 127.0.0.1 --port 8902 --strictPort",
      url: "http://127.0.0.1:8902",
      reuseExistingServer: false,
      timeout: 60_000,
    },
    {
      command: `mkdir -p /home/mfrager/tmp/mm-e2e-script && ${build} && OPENAI_BASE_URL= OPENAI_API_KEY= ${bin} --data-dir /home/mfrager/tmp/mm-e2e-script --port 8895 --controller-script crates/mm-serve/tests/fixtures/budget_focus.json`,
      cwd: "..",
      url: "http://127.0.0.1:8895/api/health",
      reuseExistingServer: false,
      timeout: 180_000,
    },
  ],
});
