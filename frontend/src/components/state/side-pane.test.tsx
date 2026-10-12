import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import { SidePane } from "@/routes/workspace.tsx";
import { applyControllerFocus, useSideStore } from "@/lib/store/side.ts";
import type { FocusDoc } from "@/lib/wire/focus.ts";

const budget: FocusDoc = {
  v: 1,
  id: "budget",
  kind: "budget",
  title: "Budgets",
  status: "saved",
  version: 1,
  source: { kind: "chat" },
  capabilities: ["refresh"],
  snapshot: { rows: [{ kind: "meta_analysis", period: "day", limit: 100, spent: 0 }] },
};

const stateBody = {
  kernel: { state: "ok", reason: null, ok_services: "sqlite, eventlog, graph, llm", llm_reason: "no language-model provider is configured", services: [] },
  being: { ok: false, reason: "the being store did not open" },
  goals: { goals: [] },
  budgets: { rows: [] },
  last_action: null,
};

beforeEach(() => {
  useSideStore.setState({ mode: "state", doc: null, error: null, label: "State", revision: 0, lastAction: null });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/api/focus/budget")) {
        return { ok: true, json: async () => budget };
      }
      if (url.includes("/api/state")) {
        return { ok: true, json: async () => stateBody };
      }
      if (url.includes("/api/health")) {
        return { ok: true, json: async () => ({ kernel: { state: "ok", services: [] }, dataDir: "/tmp/mm", eventHead: { seq: 0 } }) };
      }
      return { ok: false, status: 404, json: async () => ({ reason: "missing" }) };
    }),
  );
});

it("renders a budget focus, then the state board again", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <SidePane />
    </QueryClientProvider>,
  );
  await applyControllerFocus({ id: "budget", kind: "budget", title: "Budgets" });
  expect(await screen.findByText("meta_analysis")).toBeTruthy();
  expect(document.querySelector("[data-state-board]")).toBeNull();
  expect(screen.getByRole("button", { name: "Ask Metamind to change this." }).hasAttribute("disabled")).toBe(true);

  await applyControllerFocus({ id: "state" });
  expect(await screen.findByText("kernel ok")).toBeTruthy();
  expect(document.querySelector("[data-doc-kind='budget']")).toBeNull();
  expect(screen.queryByText("meta_analysis")).toBeNull();
});
