import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useAgentstream } from "@/hooks/use-agentstream.ts";
import { useAgentstreamStore } from "@/lib/store/agentstream.ts";
import type { Event, EventType } from "@/lib/wire/agentstream.ts";

function record(seq: number, type: EventType, data: Record<string, unknown>): Event {
  return {
    v: 2,
    seq,
    id: `e${seq}`,
    sessionId: "s1",
    turnId: "t1",
    at: 1_700_000_000_000 + seq,
    type,
    source: "test",
    data,
  };
}

const E1 = record(1, "turn.started", { text: "hello" });
const E2 = record(2, "pass.started", { pass: "interpret", attempt: 1 });
const E3 = record(3, "turn.completed", { reply: "hi", verified: true });
const E4 = record(4, "agent.message", { text: "hi", verified: true });
const E5 = record(5, "pass.completed", { pass: "verify", attempt: 1 });

class FakeEventSource {
  static instances: FakeEventSource[] = [];
  readonly url: string;
  closed = false;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  private listeners = new Map<string, Set<(event: MessageEvent) => void>>();

  constructor(url: string | URL) {
    this.url = String(url);
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: EventListener) {
    const set = this.listeners.get(type) ?? new Set();
    set.add(listener as (event: MessageEvent) => void);
    this.listeners.set(type, set);
  }

  close() {
    this.closed = true;
  }

  open() {
    this.onopen?.();
  }

  fail() {
    this.onerror?.();
  }

  emit(type: string, payload: unknown) {
    const message = new MessageEvent(type, { data: JSON.stringify(payload) });
    for (const listener of this.listeners.get(type) ?? []) listener(message);
  }
}

function page(from: number, events: Event[]) {
  return {
    sessionId: "s1",
    from,
    next: events.length ? (events.at(-1)?.seq ?? from) + 1 : from,
    events,
  };
}

function mockFetch(pages: Record<number, Event[]>) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), "http://localhost");
    const from = Number(url.searchParams.get("from") ?? "1");
    return {
      ok: true,
      status: 200,
      json: async () => page(from, pages[from] ?? []),
    };
  });
}

beforeEach(() => {
  FakeEventSource.instances = [];
  useAgentstreamStore.getState().begin("s1");
  vi.stubGlobal("EventSource", FakeEventSource);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useAgentstream", () => {
  it("loads a snapshot, opens from lastSeq + 1, and drops duplicate seqs", async () => {
    vi.stubGlobal("fetch", mockFetch({ 1: [E1, E2] }));
    const { unmount } = renderHook(() => useAgentstream("s1"));

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    const source = FakeEventSource.instances[0];
    expect(source?.url).toContain("from=3");

    act(() => {
      source?.open();
      source?.emit("turn.completed", E3);
    });
    await waitFor(() => expect(useAgentstreamStore.getState().records).toHaveLength(3));

    act(() => source?.emit("turn.completed", E3));
    expect(useAgentstreamStore.getState().records).toHaveLength(3);
    expect(useAgentstreamStore.getState().lastSeq).toBe(3);
    unmount();
  });

  it("reconnects from the last applied cursor without duplicating rows", async () => {
    vi.stubGlobal("fetch", mockFetch({ 1: [E1, E2] }));
    const { unmount } = renderHook(() => useAgentstream("s1"));

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    act(() => FakeEventSource.instances[0]?.fail());

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(2), { timeout: 2_000 });
    expect(FakeEventSource.instances[1]?.url).toContain("from=3");

    act(() => {
      FakeEventSource.instances[1]?.open();
      FakeEventSource.instances[1]?.emit("turn.completed", E3);
    });
    await waitFor(() => expect(useAgentstreamStore.getState().records).toHaveLength(3));
    act(() => FakeEventSource.instances[1]?.emit("turn.completed", E3));
    expect(useAgentstreamStore.getState().records.map((event) => event.seq)).toEqual([1, 2, 3]);
    unmount();
  });

  it("refills from the cursor when the live stream jumps a seq", async () => {
    vi.stubGlobal("fetch", mockFetch({ 1: [E1, E2], 3: [E3, E4] }));
    const { unmount } = renderHook(() => useAgentstream("s1"));

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    act(() => FakeEventSource.instances[0]?.emit("pass.completed", E5));

    await waitFor(() => expect(useAgentstreamStore.getState().records).toHaveLength(4));
    expect(useAgentstreamStore.getState().records.map((event) => event.seq)).toEqual([1, 2, 3, 4]);
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(2));
    expect(FakeEventSource.instances[1]?.url).toContain("from=5");
    unmount();
  });
});
