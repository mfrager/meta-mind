import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { initialLlmLogState, useLlmLogStore } from "@/lib/store/llm-log.ts";
import { LLM_LOG_MAX_HEIGHT, LLM_LOG_MIN_HEIGHT, usePrefsStore } from "@/lib/store/prefs.ts";
import type { LlmActivityEvent } from "@/lib/wire/llm_activity.ts";
import { LlmLogDock } from "./llm-log-dock.tsx";

const request: LlmActivityEvent = {
  kind: "request",
  seq: 1,
  at: 1_700_000_000_000,
  callId: "call-1",
  source: "chat",
  pass: "process",
  round: 2,
  trigger: "tool_result",
  tool: "memory.add",
  origin: "controller.turn",
  purpose: "converse",
  provider: "openai",
  model: "gpt-test",
  streamed: true,
  requested: true,
  messageCount: 3,
  toolCount: 2,
  preview: "hello",
  attempt: 1,
};

const delta: LlmActivityEvent = {
  kind: "delta",
  seq: 2,
  at: 1_700_000_000_100,
  callId: "call-1",
  text: "streamed reply",
  chars: 14,
  truncated: false,
};

const response: LlmActivityEvent = {
  kind: "response",
  seq: 3,
  at: 1_700_000_000_200,
  callId: "call-1",
  model: "gpt-test",
  status: "ok",
  cached: false,
  tokensIn: 20,
  tokensOut: 8,
  latencyMs: 120,
  text: "final answer",
  toolCalls: 1,
  toolPreview: "memory.add {}",
  streamText: "final answer",
  streamChars: 12,
  streamTruncated: false,
  reasoningChars: 0,
  attempt: 1,
};

const reasoningOnly: LlmActivityEvent = {
  kind: "error",
  seq: 3,
  at: 1_700_000_000_200,
  callId: "call-1",
  status: "reasoning_only",
  error: "llm reasoning-only answer: 3838 reasoning chars, no visible text (finish_reason: length)",
  latencyMs: 2400,
  partialChars: 0,
  streamChars: 0,
  reasoningChars: 3838,
  finishReason: "length",
  attempt: 1,
};

const retry: LlmActivityEvent = {
  ...request,
  seq: 4,
  at: 1_700_000_001_000,
  callId: "call-2",
  parentCallId: "call-1",
  trigger: "budget_retry",
  attempt: 2,
  tool: undefined,
};

const retryResponse: LlmActivityEvent = {
  ...response,
  seq: 5,
  at: 1_700_000_011_000,
  callId: "call-2",
  latencyMs: 8100,
  streamText: "The migration order is fixed by the schema; apply 003 then 004.",
  streamChars: 64,
  attempt: 2,
};

function seed(events: LlmActivityEvent[]) {
  useLlmLogStore.getState().resync({
    oldestSeq: events[0]?.seq ?? 1,
    nextSeq: (events[events.length - 1]?.seq ?? 0) + 1,
    events,
  });
  useLlmLogStore.setState({ connection: "live" });
}

describe("LlmLogDock", () => {
  beforeEach(() => {
    useLlmLogStore.setState({ ...initialLlmLogState });
    usePrefsStore.setState({ llmLogCollapsed: false, llmLogHeight: 168, llmLogTab: "log" });
  });

  it("shows a loading state before the first snapshot", () => {
    useLlmLogStore.setState({ connection: "loading" });
    render(<LlmLogDock />);
    expect(screen.getByText("Loading recent activity…")).toBeTruthy();
  });

  it("names an interrupted connection instead of showing a stale empty feed", () => {
    useLlmLogStore.setState({ connection: "reconnecting" });
    render(<LlmLogDock />);
    expect(screen.getByText("Reconnecting to the activity feed…")).toBeTruthy();
  });

  it("shows an explicit empty state", () => {
    useLlmLogStore.setState({ connection: "live" });
    render(<LlmLogDock />);
    expect(screen.getByText("No model activity yet.")).toBeTruthy();
  });

  it("shows the backend reason when the feed is unavailable", () => {
    useLlmLogStore.setState({
      connection: "error",
      error: "LLM activity is disabled (MM_LLM_ACTIVITY).",
    });
    render(<LlmLogDock />);
    expect(screen.getByRole("alert").textContent).toContain("MM_LLM_ACTIVITY");
  });

  it("renders retained request, stream and response rows with attribution", () => {
    seed([request, delta, response]);
    render(<LlmLogDock />);

    expect(screen.getByText("REQ")).toBeTruthy();
    expect(screen.getByText("RSP")).toBeTruthy();
    expect(screen.getByText("streamed reply")).toBeTruthy();
    expect(screen.getByText("final answer")).toBeTruthy();
    expect(
      screen.getByText(/chat\/process r2 · converse · trigger=tool_result · tool=memory\.add/),
    ).toBeTruthy();
    expect(screen.getByText(/20\/8 tok · 120 ms/)).toBeTruthy();
    expect(screen.queryByText("STR")).toBeNull();
  });

  it("rolls the full stream text up into the rsp line", () => {
    seed([request, delta, response]);
    render(<LlmLogDock />);

    const rsp = screen.getByText(/20\/8 tok · 120 ms/).closest("[data-kind]");
    expect(rsp?.getAttribute("data-kind")).toBe("response");
    expect(rsp?.textContent).toContain("final answer");
  });

  it("shows the reply a tool call carried instead of an empty reply", () => {
    const approval: LlmActivityEvent = {
      ...response,
      text: "",
      toolCalls: 1,
      toolPreview: 'metamind.approve {"success":true,"reply":"Hello — how can I help?"}',
      streamText: "",
      streamChars: 0,
      reasoningChars: 201,
    };
    seed([request, approval]);
    render(<LlmLogDock />);

    expect(screen.queryByText("(empty reply)")).toBeNull();
    expect(screen.getByText(/metamind\.approve .*Hello — how can I help\?/)).toBeTruthy();
    expect(screen.getByText(/201 reasoning chars · 1 tool calls/)).toBeTruthy();
    expect(screen.queryByText(/0 chars/)).toBeNull();
  });

  it("shows every tool call even when the reply has text", () => {
    const multi: LlmActivityEvent = {
      ...response,
      toolCalls: 2,
      toolPreview: 'data.query {"sql":"select 1"}; conversation.status {"action":"read"}',
    };
    seed([request, multi]);
    render(<LlmLogDock />);

    // The reply is the summary; the calls are their own visible line, not a
    // count the user has to expand to understand.
    expect(screen.getByText("final answer")).toBeTruthy();
    expect(screen.getByText(/2 tool calls/)).toBeTruthy();
    expect(screen.getByText(/data\.query .*conversation\.status/)).toBeTruthy();
  });

  it("nests a retry group under the request that failed", () => {
    seed([request, reasoningOnly, retry, retryResponse]);
    const { container } = render(<LlmLogDock />);

    const errorRow = container.querySelector("[data-kind='error']");
    expect(errorRow?.closest("li[data-kind='request']")?.getAttribute("data-call-id")).toBe("call-1");

    const retryRow = container.querySelector("[data-call-id='call-2'][data-kind='request']");
    expect(retryRow?.closest("ul")?.closest("li[data-kind='request']")?.getAttribute("data-call-id")).toBe(
      "call-1",
    );
    expect(screen.getByText(/trigger=budget_retry · parent=call-1/)).toBeTruthy();
    expect(screen.getByText(/The migration order is fixed by the schema/)).toBeTruthy();
  });

  it("shows a reasoning-only outcome with its count instead of a bare transport string", () => {
    seed([request, reasoningOnly]);
    render(<LlmLogDock />);

    expect(screen.getByText(/reasoning_only · length · 3838 reasoning chars/)).toBeTruthy();
  });

  it("filters groups and keeps the request as context", () => {
    seed([request, delta, response]);
    render(<LlmLogDock />);

    fireEvent.change(screen.getByLabelText("Filter activity"), { target: { value: "responses" } });
    expect(screen.getByText("REQ")).toBeTruthy();
    expect(screen.getByText("RSP")).toBeTruthy();

    fireEvent.change(screen.getByLabelText("Filter activity"), { target: { value: "requests" } });
    expect(screen.getByText("REQ")).toBeTruthy();
    expect(screen.queryByText("RSP")).toBeNull();

    fireEvent.change(screen.getByLabelText("Filter activity"), { target: { value: "errors" } });
    expect(screen.getByText("No activity matches this filter.")).toBeTruthy();
  });

  it("pauses the visible list and resumes with the buffered rows", () => {
    useLlmLogStore.getState().ingest(1, request);
    useLlmLogStore.setState({ connection: "live" });
    render(<LlmLogDock />);

    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    act(() => useLlmLogStore.getState().ingest(3, response));
    expect(screen.queryByText("RSP")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Resume (1)" }));
    expect(screen.getByText("RSP")).toBeTruthy();
  });

  it("clears the dock without moving the stream cursor", () => {
    useLlmLogStore.getState().ingest(1, request);
    useLlmLogStore.setState({ connection: "live" });
    render(<LlmLogDock />);

    fireEvent.click(screen.getByRole("button", { name: "Clear activity" }));
    expect(screen.getByText("No model activity yet.")).toBeTruthy();
    expect(useLlmLogStore.getState().lastSeq).toBe(1);
  });

  it("collapses to its header and expands again", () => {
    useLlmLogStore.getState().ingest(1, request);
    useLlmLogStore.setState({ connection: "live" });
    const { container } = render(<LlmLogDock />);
    const dock = container.querySelector("[data-llm-log-dock]");

    fireEvent.click(screen.getByRole("button", { name: "Collapse LLM activity" }));
    expect(dock?.getAttribute("data-collapsed")).toBe("true");
    expect(screen.queryByText("hello")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Expand LLM activity" }));
    expect(dock?.getAttribute("data-collapsed")).toBe("false");
    expect(screen.getByText("hello")).toBeTruthy();
  });

  it("clamps a persisted dock height to the usable range", () => {
    usePrefsStore.getState().setLlmLogHeight(10);
    expect(usePrefsStore.getState().llmLogHeight).toBe(LLM_LOG_MIN_HEIGHT);
    usePrefsStore.getState().setLlmLogHeight(10_000);
    expect(usePrefsStore.getState().llmLogHeight).toBe(LLM_LOG_MAX_HEIGHT);
  });

  it("resizes with the keyboard from the drag grip", () => {
    useLlmLogStore.setState({ connection: "live" });
    render(<LlmLogDock />);
    const grip = screen.getByRole("separator", { name: "Resize LLM activity" });

    fireEvent.keyDown(grip, { key: "ArrowUp" });
    expect(usePrefsStore.getState().llmLogHeight).toBe(184);
    fireEvent.keyDown(grip, { key: "ArrowDown" });
    expect(usePrefsStore.getState().llmLogHeight).toBe(168);
  });

  it("expands a request to reveal its ids and capped previews", () => {
    seed([request, delta, response]);
    render(<LlmLogDock />);

    const row = screen.getByText("streamed reply").closest("button");
    if (!row) throw new Error("no row button");
    expect(row.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("call-1")).toBeTruthy();
    expect(screen.getByText("hello")).toBeTruthy();
    expect(screen.getByText("prompt")).toBeTruthy();
    expect(screen.getByText("trigger")).toBeTruthy();
  });

  it("keeps height, pause, and tab state across a switch", () => {
    seed([request, response]);
    usePrefsStore.setState({ llmLogHeight: 200 });
    useLlmLogStore.setState({ paused: true, pausedCount: 3 });
    const { container } = render(<LlmLogDock />);
    const dock = container.querySelector("[data-llm-log-dock]");

    fireEvent.click(screen.getByRole("tab", { name: "Text" }));
    expect(dock?.getAttribute("data-tab")).toBe("text");
    expect(dock?.getAttribute("data-paused")).toBe("true");
    expect((dock as HTMLElement).style.height).toBe("200px");
    expect(usePrefsStore.getState().llmLogTab).toBe("text");

    fireEvent.click(screen.getByRole("tab", { name: "Log" }));
    expect(dock?.getAttribute("data-tab")).toBe("log");
    expect(usePrefsStore.getState().llmLogHeight).toBe(200);
    expect(useLlmLogStore.getState().paused).toBe(true);
  });

  it("renders exactly three lines per block in the text tab", () => {
    seed([request, delta, response]);
    usePrefsStore.setState({ llmLogTab: "text" });
    const { container } = render(<LlmLogDock />);

    expect(screen.getByLabelText("LLM activity text log")).toBeTruthy();
    const blocks = container.querySelectorAll("[data-llm-log-text-block]");
    expect(blocks).toHaveLength(2);
    for (const block of blocks) {
      expect(block.querySelectorAll("[data-llm-log-text-line]")).toHaveLength(3);
    }
    expect(container.querySelector("[data-llm-log-text-block][data-kind='request']")?.textContent).toContain(
      "chat/process r2 · converse · trigger=tool_result tool=memory.add",
    );
  });

  it("keeps the tool calls in the text tab when the reply also has text", () => {
    seed([request, delta, response]);
    usePrefsStore.setState({ llmLogTab: "text" });
    const { container } = render(<LlmLogDock />);

    const responseBlock = container.querySelector("[data-llm-log-text-block][data-kind='response']");
    expect(responseBlock?.textContent).toContain("final answer");
    expect(responseBlock?.textContent).toContain("tools: memory.add {}");
  });

  it("renders a reasoning-only error with its count in the text tab", () => {
    seed([request, reasoningOnly, retry, retryResponse]);
    usePrefsStore.setState({ llmLogTab: "text" });
    const { container } = render(<LlmLogDock />);

    const errorBlock = container.querySelector("[data-llm-log-text-block][data-kind='error']");
    expect(errorBlock?.textContent).toContain("reasoning_only · length · 3838 reasoning chars");
    expect(errorBlock?.textContent).toContain("↳ERR");
    expect(errorBlock?.getAttribute("data-depth")).toBe("1");
  });

  it("never creates a text block from a delta frame", () => {
    seed([request, delta, response]);
    usePrefsStore.setState({ llmLogTab: "text" });
    const { container } = render(<LlmLogDock />);

    const kinds = Array.from(container.querySelectorAll("[data-llm-log-text-block]")).map((block) =>
      block.getAttribute("data-kind"),
    );
    expect(kinds).toEqual(["request", "response"]);
    expect(container.textContent).not.toContain("STR");
  });

  it("copies the untruncated text log", () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    const longReply = `line one\nline two   with   spaces ${"tail ".repeat(400)}`;
    seed([request, delta, { ...response, streamText: longReply, streamChars: longReply.length }]);
    usePrefsStore.setState({ llmLogTab: "text" });
    render(<LlmLogDock />);

    fireEvent.click(screen.getByRole("button", { name: "Copy text log" }));
    expect(writeText).toHaveBeenCalledTimes(1);
    const payload = String(writeText.mock.calls[0]?.[0] ?? "");
    expect(payload).toContain('"line one line two with spaces');
    expect(payload).toContain("tail ".repeat(400).trim());
    expect(payload.split("\n")).toHaveLength(6);
  });
});
