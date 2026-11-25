import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { initialLlmLogState, useLlmLogStore } from "@/lib/store/llm-log.ts";
import type { LlmActivityEvent } from "@/lib/wire/llm_activity.ts";
import { RECONNECT_MS, useLlmActivityStream } from "./use-llm-activity-stream.ts";

class FakeEventSource {
  static list: FakeEventSource[] = [];

  readonly url: string;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  private listeners = new Map<string, Set<(event: MessageEvent) => void>>();

  constructor(url: string | URL) {
    this.url = String(url);
    FakeEventSource.list.push(this);
  }

  addEventListener(type: string, listener: (event: MessageEvent) => void) {
    const set = this.listeners.get(type) ?? new Set();
    set.add(listener);
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

  emit(type: string, data: unknown) {
    for (const listener of this.listeners.get(type) ?? []) {
      listener({ data: JSON.stringify(data) } as MessageEvent);
    }
  }
}

function Harness() {
  useLlmActivityStream();
  return null;
}

const retained: LlmActivityEvent = {
  kind: "request",
  seq: 1,
  at: 1_700_000_000_000,
  callId: "call-1",
  source: "chat",
  purpose: "reply",
  provider: "openai",
  streamed: true,
  requested: true,
  messageCount: 2,
  toolCount: 0,
  preview: "hello",
  attempt: 1,
};

const live: LlmActivityEvent = {
  kind: "response",
  seq: 2,
  at: 1_700_000_000_100,
  callId: "call-1",
  model: "gpt-test",
  status: "ok",
  cached: false,
  tokensIn: 4,
  tokensOut: 2,
  latencyMs: 30,
  text: "hi",
  toolCalls: 0,
  toolPreview: "",
  streamText: "hi",
  streamChars: 2,
  streamTruncated: false,
  reasoningChars: 0,
  attempt: 1,
};

describe("useLlmActivityStream", () => {
  beforeEach(() => {
    useLlmLogStore.setState({ ...initialLlmLogState });
    FakeEventSource.list = [];
    vi.stubGlobal("EventSource", FakeEventSource);
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        ok: true,
        status: 200,
        json: async () => ({ oldestSeq: 1, nextSeq: 2, events: [retained] }),
      })),
    );
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("hydrates from a snapshot and follows the live tail", async () => {
    render(<Harness />);
    await waitFor(() => expect(FakeEventSource.list).toHaveLength(1));
    const source = FakeEventSource.list[0];
    if (!source) throw new Error("no source");
    expect(source.url).toContain("from=2");
    expect(useLlmLogStore.getState().rows).toHaveLength(1);

    act(() => source.open());
    expect(useLlmLogStore.getState().connection).toBe("live");

    act(() => source.emit("activity", live));
    await waitFor(() => expect(useLlmLogStore.getState().rows).toHaveLength(2));
    expect(useLlmLogStore.getState().rows[1]?.kind).toBe("response");
  });

  it("reconnects from the last seen cursor after an error", async () => {
    render(<Harness />);
    await waitFor(() => expect(FakeEventSource.list).toHaveLength(1));
    const source = FakeEventSource.list[0];
    if (!source) throw new Error("no source");
    act(() => source.open());

    vi.useFakeTimers();
    act(() => source.fail());
    expect(useLlmLogStore.getState().connection).toBe("reconnecting");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RECONNECT_MS);
    });

    expect(FakeEventSource.list).toHaveLength(2);
    expect(FakeEventSource.list[1]?.url).toContain("from=2");
  });
});
