import { beforeEach, describe, expect, it } from "vitest";

import type { LlmActivityEvent } from "@/lib/wire/llm_activity.ts";

import {
  MAX_ROWS,
  applyActivityEvent,
  filterRows,
  type LlmLogRow,
  initialLlmLogState,
  rowsFromEvents,
  useLlmLogStore,
} from "./llm-log.ts";

function request(seq: number, callId = "call-1"): LlmActivityEvent {
  return {
    kind: "request",
    seq,
    at: 1_700_000_000_000 + seq,
    callId,
    source: "chat",
    purpose: "reply",
    provider: "openai",
    model: "gpt-test",
    streamed: true,
    requested: true,
    messageCount: 2,
    toolCount: 1,
    preview: "hello",
  };
}

function delta(seq: number, text: string, callId = "call-1"): LlmActivityEvent {
  return {
    kind: "delta",
    seq,
    at: 1_700_000_000_000 + seq,
    callId,
    text,
    chars: text.length,
    truncated: false,
  };
}

function response(seq: number, callId = "call-1"): LlmActivityEvent {
  return {
    kind: "response",
    seq,
    at: 1_700_000_000_000 + seq,
    callId,
    model: "gpt-test",
    status: "ok",
    cached: false,
    tokensIn: 12,
    tokensOut: 4,
    latencyMs: 90,
    text: "hi",
    toolCalls: 0,
  };
}

describe("LLM activity store", () => {
  beforeEach(() => {
    useLlmLogStore.setState({ ...initialLlmLogState });
  });

  it("folds stream fragments into their request row", () => {
    const rows = rowsFromEvents([request(1), delta(2, "he"), delta(3, "llo")]);
    expect(rows).toHaveLength(1);
    const row = rows[0];
    expect(row?.kind).toBe("request");
    if (row?.kind !== "request") return;
    expect(row.streamText).toBe("hello");
    expect(row.streamChars).toBe(3);
  });

  it("ignores duplicate and stale cursors", () => {
    const store = useLlmLogStore.getState();
    store.ingest(1, request(1));
    store.ingest(1, request(1));
    store.ingest(0, request(0));
    expect(useLlmLogStore.getState().rows).toHaveLength(1);
    expect(useLlmLogStore.getState().lastSeq).toBe(1);
  });

  it("bounds the retained rows from the oldest side", () => {
    let rows: LlmLogRow[] = [];
    for (let seq = 1; seq <= MAX_ROWS + 5; seq += 1) {
      rows = applyActivityEvent(rows, request(seq, `call-${seq}`));
    }
    expect(rows).toHaveLength(MAX_ROWS);
    expect(rows[0]?.seq).toBe(6);
  });

  it("freezes the visible list while paused and applies the buffer on resume", () => {
    const store = useLlmLogStore.getState();
    store.ingest(1, request(1));
    store.setPaused(true);
    store.ingest(2, delta(2, "later"));
    expect(useLlmLogStore.getState().rows[0]).toMatchObject({ streamText: "" });
    expect(useLlmLogStore.getState().pausedCount).toBe(1);

    useLlmLogStore.getState().setPaused(false);
    const row = useLlmLogStore.getState().rows[0];
    expect(row?.kind).toBe("request");
    if (row?.kind !== "request") return;
    expect(row.streamText).toBe("later");
    expect(useLlmLogStore.getState().pausedCount).toBe(0);
  });

  it("filters by kind without changing the retained rows", () => {
    const store = useLlmLogStore.getState();
    store.ingest(1, request(1));
    store.ingest(2, response(2));
    const rows = useLlmLogStore.getState().rows;
    expect(filterRows(rows, "requests")).toHaveLength(1);
    expect(filterRows(rows, "streams")).toHaveLength(1);
    expect(filterRows(rows, "responses")).toHaveLength(1);
    expect(filterRows(rows, "errors")).toHaveLength(0);
    store.setFilter("responses");
    expect(useLlmLogStore.getState().filter).toBe("responses");
    expect(useLlmLogStore.getState().rows).toHaveLength(2);
  });

  it("resync replaces rows and resumes from the snapshot cursor", () => {
    useLlmLogStore.getState().ingest(9, request(9));
    useLlmLogStore.getState().resync({
      oldestSeq: 3,
      nextSeq: 5,
      events: [request(3), response(4)],
    });
    const state = useLlmLogStore.getState();
    expect(state.rows).toHaveLength(2);
    expect(state.lastSeq).toBe(4);
    expect(state.oldestSeq).toBe(3);
    expect(state.hydrated).toBe(true);
    expect(state.pausedCount).toBe(0);
  });
});
