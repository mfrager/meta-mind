import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import { SidePane } from "@/routes/workspace.tsx";
import { useChatPending } from "@/lib/store/chat-pending.ts";
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
  useChatPending.setState({ text: null, propose: null });
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

it("refreshes the state board in place instead of flashing its loading state", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <SidePane />
    </QueryClientProvider>,
  );
  expect(await screen.findByText("kernel ok")).toBeTruthy();
  expect(screen.queryByText("Checking the kernel…")).toBeNull();

  // A tool asking the board to reload (or a health poll in production) refetches
  // against the same cache entry, so what is on screen stays there.
  await act(async () => {
    useSideStore.setState({ revision: 1 });
  });
  expect(screen.queryByText("Checking the kernel…")).toBeNull();
  expect(screen.getByText("kernel ok")).toBeTruthy();
});

const goals: FocusDoc = {
  v: 1,
  id: "goals",
  kind: "tree",
  title: "Goals",
  status: "saved",
  version: 1,
  source: { kind: "chat" },
  capabilities: ["edit"],
  snapshot: {},
};

it("a goal document proposes focus.patch and a fixture stays disabled", () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  useSideStore.setState({ mode: "document", doc: goals, error: null, label: "Goals", revision: 0, lastAction: null });
  const view = render(
    <QueryClientProvider client={client}>
      <SidePane />
    </QueryClientProvider>,
  );
  fireEvent.change(screen.getByRole("textbox", { name: "Goal description" }), { target: { value: "ship the console" } });
  fireEvent.click(screen.getByRole("button", { name: "Ask Metamind to change this." }));
  expect(useChatPending.getState().propose).toEqual({
    name: "focus.patch",
    input: { id: "goals", patch: { op: "set_field", field: "description", value: "ship the console" } },
  });
  expect(useChatPending.getState().text).toBe("add a goal");

  view.rerender(
    <QueryClientProvider client={client}>
      <SidePane />
    </QueryClientProvider>,
  );
  useSideStore.setState({
    mode: "document",
    doc: { ...goals, reason: "This tree is a fixture." },
    error: null,
    label: "Goals",
    revision: 0,
    lastAction: null,
  });
  view.rerender(
    <QueryClientProvider client={client}>
      <SidePane />
    </QueryClientProvider>,
  );
  const locked = screen.getByRole("button", { name: "Ask Metamind to change this." });
  expect(locked.hasAttribute("disabled")).toBe(true);
  expect(locked.getAttribute("title")).toBe("This tree is a fixture.");
});
