import { beforeEach, describe, expect, it } from "vitest";

import type { LlmActivityEvent } from "@/lib/wire/llm_activity.ts";

import {
  MAX_ROWS,
  applyActivityEvent,
  filterGroups,
  groupRows,
  type LlmLogRow,
  initialLlmLogState,
  rowsFromEvents,
  useLlmLogStore,
} from "./llm-log.ts";

function request(seq: number, callId = "call-1", extra: Record<string, unknown> = {}): LlmActivityEvent {
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
    attempt: 1,
    ...extra,
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

function response(seq: number, callId = "call-1", extra: Record<string, unknown> = {}): LlmActivityEvent {
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
    toolPreview: "",
    streamText: "hi there",
    streamChars: 8,
    streamTruncated: false,
    reasoningChars: 0,
    attempt: 1,
    ...extra,
  };
}

function error(seq: number, callId = "call-1", extra: Record<string, unknown> = {}): LlmActivityEvent {
  return {
    kind: "error",
    seq,
    at: 1_700_000_000_000 + seq,
    callId,
    status: "reasoning_only",
    error: "llm reasoning-only answer: 3838 reasoning chars, no visible text (finish_reason: length)",
    latencyMs: 2400,
    partialChars: 0,
    streamChars: 0,
    reasoningChars: 3838,
    finishReason: "length",
    attempt: 1,
    ...extra,
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

describe("groupRows", () => {
  it("nests an outcome under its request", () => {
    const groups = groupRows(rowsFromEvents([request(1), response(2)]));
    expect(groups).toHaveLength(1);
    expect(groups[0]?.request.callId).toBe("call-1");
    expect(groups[0]?.outcome?.kind).toBe("response");
    expect(groups[0]?.children).toEqual([]);
  });

  it("nests a retry group under its parent request", () => {
    const retry = request(3, "call-2", {
      parentCallId: "call-1",
      trigger: "budget_retry",
      attempt: 2,
    });
    const groups = groupRows(
      rowsFromEvents([request(1), error(2), retry, response(4, "call-2")]),
    );

    expect(groups).toHaveLength(1);
    expect(groups[0]?.outcome?.kind).toBe("error");
    expect(groups[0]?.children.map((child) => child.callId)).toEqual(["call-2"]);
    expect(groups[0]?.children[0]?.request.trigger).toBe("budget_retry");
    expect(groups[0]?.children[0]?.outcome?.kind).toBe("response");
  });

  it("flattens a chain deeper than two levels onto its oldest ancestor", () => {
    const retry = request(2, "call-2", { parentCallId: "call-1", attempt: 2 });
    const third = request(3, "call-3", { parentCallId: "call-2", attempt: 2 });
    const groups = groupRows(rowsFromEvents([request(1), retry, third]));

    expect(groups).toHaveLength(1);
    expect(groups[0]?.children.map((child) => child.callId)).toEqual(["call-2", "call-3"]);
    expect(groups[0]?.children[0]?.children).toEqual([]);
  });

  it("renders an evicted parent as a marked orphan root", () => {
    const retry = request(1, "call-2", { parentCallId: "call-gone", attempt: 2 });
    const groups = groupRows(rowsFromEvents([retry, response(2, "call-2")]));

    expect(groups).toHaveLength(1);
    expect(groups[0]?.orphan).toBe(true);
    expect(groups[0]?.outcome?.kind).toBe("response");
  });

  it("treats a cyclic parent link as an orphan instead of looping", () => {
    const first = request(1, "call-1", { parentCallId: "call-2" });
    const second = request(2, "call-2", { parentCallId: "call-1" });
    const groups = groupRows(rowsFromEvents([first, second]));

    expect(groups).toHaveLength(2);
    expect(groups.every((group) => group.orphan && group.children.length === 0)).toBe(true);
  });
});

describe("filterGroups", () => {
  const groups = () =>
    groupRows(
      rowsFromEvents([
        request(1),
        response(2),
        request(3, "call-2", { parentCallId: "call-1", attempt: 2 }),
        error(4, "call-2"),
        request(5, "call-3"),
      ]),
    );

  it("keeps the request as context for responses and errors", () => {
    const responses = filterGroups(groups(), "responses");
    expect(responses).toHaveLength(1);
    expect(responses[0]?.request.callId).toBe("call-1");
    expect(responses[0]?.children[0]?.callId).toBe("call-2");

    const errors = filterGroups(groups(), "errors");
    expect(errors).toHaveLength(1);
    expect(errors[0]?.children[0]?.outcome?.kind).toBe("error");
  });

  it("hides outcomes for the requests filter without dropping the group", () => {
    const requests = filterGroups(groups(), "requests");
    expect(requests).toHaveLength(2);
    expect(requests.every((group) => group.outcome === null && group.children.length === 0)).toBe(true);
  });

  it("filters streams by the request's streamed flag", () => {
    const rows = [request(1, "call-1", { streamed: false }), request(2, "call-2", { streamed: true })];
    expect(filterGroups(groupRows(rowsFromEvents(rows)), "streams").map((group) => group.callId)).toEqual([
      "call-2",
    ]);
  });
});
