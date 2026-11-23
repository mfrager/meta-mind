import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

import { statusFixture } from "@/lib/inspector/__fixtures__/status.ts";
import { StatusPanel } from "./status-panel.tsx";

const board = {
  kernel: { state: "ok", services: [{ name: "sqlite", state: "ok" }] },
  being: {
    ok: true,
    self_description: "A local kernel",
    version: "3",
    invariants: [],
    goal_count: 2,
    voice: { warmth: 0.7, directness: 0.6, curiosity: 0.8, playfulness: 0.3, formality: 0.4 },
  },
  goals: { goals: [] },
  budgets: { rows: [] },
  last_action: null,
};

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe("StatusPanel", () => {
  beforeEach(() => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({ ok: true, status: 200, json: async () => board }) as Response),
    );
  });

  it("renders the topic anchor, session, prompt and budget state", () => {
    const view = statusFixture();
    view.state.topic = null;
    view.state.status = {};
    render(<StatusPanel view={view} />, { wrapper });

    expect(screen.getByText("topic not set")).toBeTruthy();
    expect(screen.getByText("depth 2")).toBeTruthy();
    expect(screen.getByText("not built for this session")).toBeTruthy();
    expect(screen.getByText(/budget ledger is not attached/)).toBeTruthy();
    expect(screen.getByText("Session")).toBeTruthy();
    expect(screen.getByText("Frame")).toBeTruthy();
    expect(screen.getByText("Tasks & next")).toBeTruthy();
  });

  it("shows explicit not-set values instead of fabricated tasks", () => {
    const view = statusFixture();
    view.state.status = {};
    render(<StatusPanel view={view} />, { wrapper });
    const tasks = screen.getAllByText("not set");
    expect(tasks.length).toBeGreaterThanOrEqual(2);
    expect(screen.queryByText("reproduce the flake")).toBeNull();
  });

  it("renders an unavailable row for a failed section", () => {
    const view = statusFixture();
    view.unavailable = { plans: "plan registry unavailable: closed" };
    render(<StatusPanel view={view} />, { wrapper });
    expect(screen.getByText(/plan registry unavailable: closed/)).toBeTruthy();
  });
});
