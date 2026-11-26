import { beforeEach, describe, expect, it } from "vitest";

import {
  filterRecords,
  groupRecords,
  matchesFilter,
  useAgentstreamStore,
} from "@/lib/store/agentstream.ts";
import type { Event, EventType } from "@/lib/wire/agentstream.ts";

function record(seq: number, type: EventType, data: Record<string, unknown>, turnId = "t1"): Event {
  return {
    v: 2,
    seq,
    id: `e${seq}`,
    sessionId: "s1",
    turnId,
    at: 1_700_000_000_000 + seq,
    type,
    source: "test",
    data,
  };
}

beforeEach(() => {
  useAgentstreamStore.getState().begin("s1");
});

describe("agentstream store", () => {
  it("drops duplicate and late seqs but advances the cursor", () => {
    const store = useAgentstreamStore.getState();
    store.ingest([record(1, "turn.started", {}), record(2, "pass.started", { pass: "interpret" })]);
    store.ingest([record(2, "pass.started", { pass: "interpret" }), record(3, "agent.message", { text: "hi" })]);

    expect(useAgentstreamStore.getState().records.map((event) => event.seq)).toEqual([1, 2, 3]);
    expect(useAgentstreamStore.getState().lastSeq).toBe(3);
  });

  it("buffers records while paused and flushes them on resume", () => {
    const store = useAgentstreamStore.getState();
    store.setPaused(true);
    store.ingest([record(1, "turn.started", {}), record(2, "agent.message", { text: "hi" })]);

    expect(useAgentstreamStore.getState().records).toHaveLength(0);
    expect(useAgentstreamStore.getState().pending).toHaveLength(2);
    expect(useAgentstreamStore.getState().lastSeq).toBe(2);

    store.setPaused(false);
    expect(useAgentstreamStore.getState().records.map((event) => event.seq)).toEqual([1, 2]);
    expect(useAgentstreamStore.getState().pending).toHaveLength(0);
  });

  it("clear empties rows without rewinding the cursor", () => {
    const store = useAgentstreamStore.getState();
    store.ingest([record(1, "turn.started", {}), record(2, "agent.message", { text: "hi" })]);
    store.clear();

    expect(useAgentstreamStore.getState().records).toHaveLength(0);
    expect(useAgentstreamStore.getState().lastSeq).toBe(2);
  });

  it("merges live deltas by call id", () => {
    const store = useAgentstreamStore.getState();
    store.ingest([
      record(1, "llm.delta", { callId: "c1", index: 0, text: "hel" }),
      record(1, "llm.delta", { callId: "c1", index: 1, text: "lo" }),
    ]);

    expect(useAgentstreamStore.getState().deltas.c1).toBe("hello");
    expect(useAgentstreamStore.getState().records).toHaveLength(0);
  });

  it("groups Turn → Pass and filters by kind", () => {
    const records = [
      record(1, "turn.started", {}),
      record(2, "pass.started", { pass: "interpret" }),
      record(3, "llm.request", { pass: "interpret", callId: "c1" }),
      record(4, "tool.call", { pass: "interpret", name: "conversation.set_topic" }),
      record(5, "verification.result", { pass: true, label: "topic" }),
    ];

    const groups = groupRecords(records);
    expect(groups).toHaveLength(1);
    expect(groups[0]?.passes.map((pass) => pass.pass)).toEqual(["", "interpret"]);
    expect(filterRecords(records, "calls").map((event) => event.type)).toEqual(["llm.request"]);
    expect(filterRecords(records, "tools").map((event) => event.type)).toEqual(["tool.call"]);
    expect(matchesFilter(record(6, "internal.error", {}), "errors")).toBe(true);
    expect(matchesFilter(record(7, "prompt.rendered", {}), "internals")).toBe(true);
  });
});
