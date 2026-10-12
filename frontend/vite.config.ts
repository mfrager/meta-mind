import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vitest/config";

// The dev server proxies /api to the local mm-serve process (127.0.0.1:8899),
// the single writer of the data directory (plan `01` §5.1).
const apiPort = process.env.MM_UI_PORT ?? "8899";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  server: {
    host: "0.0.0.0",
    port: 8900,
    proxy: {
      "/api": { target: `http://127.0.0.1:${apiPort}`, changeOrigin: false },
    },
  },
  preview: {
    host: "0.0.0.0",
    port: 4173,
  },
  test: {
    environment: "happy-dom",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    setupFiles: ["src/test-setup.ts"],
  },
});
