import { describe, expect, it } from "vitest";

import { applyEvent, settlePasses } from "@/agentgui/lib/store/chat-store";
import { eventsForFrame } from "@/agentgui/services/agent-transport";
import type { AgentEvent, ChatMessage, PassPart } from "@/agentgui/types/workspace";
import type { ChatFrame } from "@/lib/wire/chat";

function assistant(): ChatMessage {
  return {
    id: "m1",
    role: "assistant",
    createdAt: new Date(0).toISOString(),
    status: "streaming",
    parts: [],
  };
}

function reduce(events: AgentEvent[]): ChatMessage {
  return events.reduce(applyEvent, assistant());
}

describe("pass rendering", () => {
  it("keeps phase prose out of the assistant answer", () => {
    const message = reduce([
      { type: "pass_start", pass: "process", attempt: 1 },
      { type: "pass_round", pass: "process", attempt: 1, round: 1, streaming: true },
      { type: "pass_delta", pass: "process", attempt: 1, round: 1, delta: "I will " },
      { type: "pass_delta", pass: "process", attempt: 1, round: 1, delta: "write it." },
      {
        type: "pass_round_end",
        pass: "process",
        attempt: 1,
        round: 1,
        outcome: "ok",
        tokensIn: 10,
        tokensOut: 4,
      },
      { type: "text", delta: "Done." },
    ]);

    expect(message.parts.filter((part) => part.type === "text")).toEqual([
      { type: "text", text: "Done." },
    ]);
    const pass = message.parts.find((part) => part.type === "pass") as PassPart;
    expect(pass.rounds[0].text).toBe("I will write it.");
    expect(pass.rounds[0].outcome).toBe("ok");
    expect(pass.rounds[0].streaming).toBe(false);
  });

  it("attaches tools and checks to the pass that produced them", () => {
    const message = reduce([
      { type: "pass_start", pass: "verify", attempt: 2 },
      {
        type: "verification",
        label: "cites",
        pass: false,
        confidence: 0.3,
        attempt: 2,
        backend: "llm_judge",
        reasoning: "no source",
      },
      {
        type: "tool",
        call: {
          id: "call-1",
          name: "data.search",
          status: "running",
          pass: "verify",
          attempt: 2,
          round: 1,
        },
      },
      {
        type: "tool",
        call: {
          id: "call-1",
          name: "data.search",
          status: "done",
          output: "found",
          pass: "verify",
          attempt: 2,
          round: 1,
        },
      },
    ]);

    const pass = message.parts.find((part) => part.type === "pass") as PassPart;
    expect(pass.attempt).toBe(2);
    expect(pass.checks).toEqual([
      {
        label: "cites",
        pass: false,
        confidence: 0.3,
        backend: "llm_judge",
        reasoning: "no source",
        durationMs: undefined,
      },
    ]);
    expect(pass.tools[0]).toMatchObject({
      id: "call-1",
      status: "done",
      output: "found",
    });
    expect(message.parts.some((part) => part.type === "trace")).toBe(false);
  });

  it("settles an open pass as cancelled, never as done", () => {
    const started = reduce([
      { type: "pass_start", pass: "process", attempt: 1 },
      { type: "pass_round", pass: "process", attempt: 1, round: 1, streaming: true },
    ]);
    const cancelled = settlePasses(started.parts, "cancelled");
    expect((cancelled[0] as PassPart).status).toBe("cancelled");
  });

  it("marks a finished failed pass and settles a dropped stream", () => {
    const failed = reduce([
      { type: "pass_start", pass: "process", attempt: 1 },
      { type: "pass_complete", pass: "process", attempt: 1, finished: "error" },
    ]);
    const part = failed.parts[0] as PassPart;
    expect(part.status).toBe("failed");

    const dropped = reduce([
      { type: "pass_start", pass: "process", attempt: 1 },
      { type: "done" },
    ]);
    expect((dropped.parts[0] as PassPart).status).toBe("done");
    expect(dropped.status).toBe("complete");
  });
});

describe("eventsForFrame pass frames", () => {
  it("never turns a final pass token into assistant text", () => {
    const frame: ChatFrame = {
      type: "pass_token",
      pass: "interpret",
      token: "Hello!",
      final: true,
    };
    expect(eventsForFrame(frame)).toEqual([]);
  });

  it("maps prose deltas and tool attribution", () => {
    const delta = eventsForFrame({
      type: "pass_token",
      pass: "process",
      token: "thinking",
      final: false,
      attempt: 2,
      round: 3,
      channel: "prose",
    });
    expect(delta).toEqual([
      {
        type: "pass_delta",
        pass: "process",
        attempt: 2,
        round: 3,
        delta: "thinking",
      },
    ]);

    const tool = eventsForFrame({
      type: "tool_call",
      id: "c1",
      turn_id: "t1",
      phase: "act",
      name: "file.write",
      input: { action: "write" },
      pass: "process",
      attempt: 2,
      round: 3,
      action: "write",
    });
    expect(tool[0]).toMatchObject({
      type: "tool",
      call: {
        id: "c1",
        pass: "process",
        attempt: 2,
        round: 3,
        action: "write",
      },
    });
  });
});
