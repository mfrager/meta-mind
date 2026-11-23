import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

import type { SessionPage, SessionSummary } from "@/lib/wire/inspector.ts";
import { SessionList } from "./session-list.tsx";

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

function session(id: string, topic: string): SessionSummary {
  return {
    sessionId: id,
    userId: "mmd:current-user",
    topic,
    lifecycle: "active",
    presence: "present",
    working: false,
    traceCount: 1,
    traceKinds: ["main"],
    lastActivityAt: Date.now(),
    sessionDir: null,
    resumable: false,
    deleted: false,
  };
}

function page(body: SessionPage) {
  return Promise.resolve({
    ok: true,
    status: 200,
    json: async () => body,
  } as Response);
}

describe("SessionList", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("renders an empty RDF list without a sample row", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => page({ items: [], nextCursor: null, hasMore: false })),
    );
    render(<SessionList onSelect={() => undefined} selectedId={null} />, { wrapper });
    expect(await screen.findByText(/No sessions yet/)).toBeTruthy();
    expect(screen.queryByText(/untitled/)).toBeNull();
  });

  it("pages with the server cursor instead of loading every session", async () => {
    const calls: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        calls.push(url);
        if (!url.includes("cursor=")) {
          return page({
            items: [session("01A", "first session")],
            nextCursor: "next-page",
            hasMore: true,
          });
        }
        return page({
          items: [session("01B", "second session")],
          nextCursor: null,
          hasMore: false,
        });
      }),
    );
    render(<SessionList onSelect={() => undefined} selectedId="01A" />, { wrapper });
    expect(await screen.findByText("first session")).toBeTruthy();
    fireEvent.click(screen.getByText("Load more sessions"));
    expect(await screen.findByText("second session")).toBeTruthy();
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
    expect(calls.some((url) => url.includes("cursor=next-page"))).toBe(true);
  });
});
