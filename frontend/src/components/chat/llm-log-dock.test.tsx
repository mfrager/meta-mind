import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

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
  purpose: "reply",
  provider: "openai",
  model: "gpt-test",
  streamed: true,
  requested: true,
  messageCount: 3,
  toolCount: 2,
  preview: "hello",
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
};

describe("LlmLogDock", () => {
  beforeEach(() => {
    useLlmLogStore.setState({ ...initialLlmLogState });
    usePrefsStore.setState({ llmLogCollapsed: false, llmLogHeight: 168 });
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

  it("renders retained request, stream and response rows", () => {
    useLlmLogStore.getState().resync({
      oldestSeq: 1,
      nextSeq: 4,
      events: [request, delta, response],
    });
    useLlmLogStore.setState({ connection: "live" });
    render(<LlmLogDock />);

    expect(screen.getByText("REQ")).toBeTruthy();
    expect(screen.getByText("RSP")).toBeTruthy();
    expect(screen.getByText("streamed reply")).toBeTruthy();
    expect(screen.getByText("final answer")).toBeTruthy();
    expect(screen.getByText(/chat · reply · gpt-test · 3 msgs · 2 tools/)).toBeTruthy();
    expect(screen.getByText(/20\/8 tok · 120 ms/)).toBeTruthy();
    expect(screen.queryByText("STR")).toBeNull();
  });

  it("filters the rows and explains an empty filter", () => {
    useLlmLogStore.getState().resync({
      oldestSeq: 1,
      nextSeq: 4,
      events: [request, delta, response],
    });
    useLlmLogStore.setState({ connection: "live" });
    render(<LlmLogDock />);

    fireEvent.change(screen.getByLabelText("Filter activity"), { target: { value: "responses" } });
    expect(screen.queryByText("REQ")).toBeNull();
    expect(screen.getByText("RSP")).toBeTruthy();

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

  it("expands a row to reveal its ids and capped previews", () => {
    useLlmLogStore.getState().resync({
      oldestSeq: 1,
      nextSeq: 4,
      events: [request, delta, response],
    });
    useLlmLogStore.setState({ connection: "live" });
    render(<LlmLogDock />);

    const row = screen.getByText("streamed reply").closest("button");
    if (!row) throw new Error("no row button");
    expect(row.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("call-1")).toBeTruthy();
    expect(screen.getByText("hello")).toBeTruthy();
    expect(screen.getByText("prompt")).toBeTruthy();
  });
});
