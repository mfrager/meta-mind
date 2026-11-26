import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { CallViewer } from "@/components/agentstream/CallViewer.tsx";
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

const request = record(10, "llm.request", {
  callId: "c1",
  pass: "process",
  attempt: 2,
  round: 1,
  provider: "mock",
  purpose: "converse",
  model: "small",
  messageCount: 3,
  toolCount: 2,
  messages: [
    { role: "system", content: "You are Metamind. FULL SYSTEM PROMPT." },
    {
      role: "assistant",
      content: "working",
      tool_calls: [{ id: "tc1", name: "conversation.set_topic", arguments: '{"topic":"incident"}' }],
    },
    { role: "tool", content: "staged", name: "conversation.set_topic", tool_call_id: "tc1" },
  ],
  tools: [
    {
      name: "conversation.set_topic",
      description: "Set the conversation topic.",
      parameters: { type: "object", properties: { topic: { type: "string" } }, required: ["topic"] },
    },
    { name: "conversation.set_depth", description: "Set the depth.", parameters: { type: "object" } },
  ],
});

const records: Event[] = [
  record(9, "prompt.rendered", {
    pass: "process",
    strategy: "cm.process.v1",
    hash: "abc123hash",
    variables: { user_input: "hello" },
    messageCount: 3,
  }),
  request,
  record(11, "llm.response", {
    callId: "c1",
    status: "ok",
    model: "small",
    text: "final pass text",
    toolCalls: [{ id: "tc1", name: "conversation.set_topic", arguments: '{"topic":"incident"}' }],
    usage: { tokensIn: 12, tokensOut: 7 },
    latencyMs: 42,
    finishReason: "stop",
  }),
  record(12, "tool.call", {
    callId: "tc1",
    name: "conversation.set_topic",
    arguments: '{"topic":"incident"}',
    pass: "process",
    attempt: 2,
    round: 1,
  }),
  record(13, "tool.result", {
    callId: "tc1",
    name: "conversation.set_topic",
    ok: true,
    output: "staged",
    pass: "process",
    attempt: 2,
    round: 1,
  }),
  record(14, "state.updated", { revision: 3, before: { topic: null }, after: { topic: "incident" } }),
  record(15, "verification.result", {
    label: "topic",
    backend: "llm_judge",
    pass: true,
    reasoning: "matches the request",
  }),
  record(16, "internal.action", { pass: "process", component: "pipeline", action: "state.stage", ok: true }),
  record(17, "llm.attempt", { callId: "c1", attemptNo: 1, status: "ok", latencyMs: 42 }),
];

describe("CallViewer", () => {
  beforeEach(() => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  });

  it("renders every message and every offered tool schema from the record", () => {
    render(<CallViewer request={request} records={records} />);

    expect(screen.getByText(/FULL SYSTEM PROMPT/)).toBeTruthy();
    expect(screen.getByText(/"topic":"incident"/)).toBeTruthy();
    expect(screen.getByText("staged")).toBeTruthy();

    fireEvent.click(screen.getByRole("tab", { name: "Tools" }));
    expect(screen.getByText("Set the conversation topic.")).toBeTruthy();
    expect(screen.getByText("Set the depth.")).toBeTruthy();
    expect(screen.getByText(/"required"/)).toBeTruthy();
    expect(screen.getByText("called")).toBeTruthy();

    fireEvent.click(screen.getByRole("tab", { name: "Response" }));
    expect(screen.getByText("final pass text")).toBeTruthy();
    expect(screen.getByText(/finish=stop/)).toBeTruthy();
    expect(screen.getByText(/tokens 12\/7/)).toBeTruthy();

    fireEvent.click(screen.getByRole("tab", { name: "Internals" }));
    expect(screen.getByText(/abc123hash/)).toBeTruthy();
    expect(screen.getByText(/matches the request/)).toBeTruthy();
    expect(screen.getByText(/state.stage/)).toBeTruthy();
  });

  it("copies the untouched record payload as JSON", async () => {
    render(<CallViewer request={request} records={records} />);
    fireEvent.click(screen.getByRole("button", { name: "Copy JSON" }));

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(JSON.stringify(request.data, null, 2));
  });
});
