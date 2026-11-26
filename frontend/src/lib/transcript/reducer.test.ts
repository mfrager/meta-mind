import { describe, expect, it } from "vitest";

import { deriveTranscript } from "@/lib/transcript/reducer.ts";
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

describe("deriveTranscript", () => {
  it("derives the user bubble, pass progress, and assistant bubble from records", () => {
    const items = deriveTranscript([
      record(1, "turn.started", { text: "hello", userInput: "hello" }),
      record(2, "pass.started", { pass: "interpret", attempt: 1 }),
      record(3, "llm.request", { pass: "interpret", attempt: 1, callId: "c1" }),
      record(4, "llm.response", { callId: "c1", text: "fast path", model: "small" }),
      record(5, "pass.completed", { pass: "interpret", attempt: 1, finished: "fast", durationMs: 12 }),
      record(6, "agent.message", { text: "Hi there", verified: true }),
    ]);

    expect(items[0]).toEqual({ kind: "user", id: "t1", text: "hello" });
    expect(items[1]).toMatchObject({ kind: "passes", id: "t1" });
    if (items[1]?.kind === "passes") {
      expect(items[1].passes[0]).toMatchObject({
        pass: "interpret",
        text: "fast path",
        status: "completed",
        model: "small",
        durationMs: 12,
      });
    }
    expect(items[2]).toEqual({ kind: "assistant", id: "t1", text: "Hi there", verified: true });
  });

  it("ignores a duplicate seq", () => {
    const events = [
      record(1, "turn.started", { text: "hello", userInput: "hello" }),
      record(2, "agent.message", { text: "Hi", verified: false }),
    ];
    const once = deriveTranscript(events);
    const twice = deriveTranscript([...events, ...events]);
    expect(twice).toEqual(once);
  });

  it("shows the live delta text for an open call", () => {
    const items = deriveTranscript(
      [
        record(1, "turn.started", { text: "hello", userInput: "hello" }),
        record(2, "llm.request", { pass: "process", attempt: 1, callId: "c9" }),
      ],
      { c9: "streaming now" },
    );
    const passes = items.find((item) => item.kind === "passes");
    expect(passes?.kind === "passes" ? passes.passes[0]?.text : undefined).toBe("streaming now");
    expect(passes?.kind === "passes" ? passes.passes[0]?.streaming : undefined).toBe(true);
  });

  it("renders a failed turn as an error item instead of a silent success", () => {
    const items = deriveTranscript([
      record(1, "turn.started", { text: "do it", userInput: "do it" }),
      record(2, "turn.failed", { error: "verification exhausted", kind: "VERIFY_EXHAUSTED" }),
    ]);
    expect(items.at(-1)).toEqual({
      kind: "error",
      id: "t1",
      message: "verification exhausted",
      detail: "VERIFY_EXHAUSTED",
    });
  });
});
