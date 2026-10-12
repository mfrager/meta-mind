import decideRun from "@/lib/transcript/fixtures/chat_run_decide.jsonl?raw";
import opsRun from "@/lib/transcript/fixtures/chat_run_ops.jsonl?raw";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ButtonGrid } from "@/components/chat/button-grid.tsx";
import { Composer } from "@/components/chat/composer.tsx";
import { ImageChooser } from "@/components/chat/image-chooser.tsx";
import { TranscriptView } from "@/components/chat/transcript-view.tsx";
import type { ChatFrame } from "@/lib/wire/chat.ts";
import { initialTranscript, reduceFrames, reduceTranscript, type SequencedFrame } from "@/lib/transcript/reducer.ts";
import { visibleItems } from "@/lib/transcript/selectors.ts";

const FIXTURES = [
  ["chat_run_decide.jsonl", decideRun],
  ["chat_run_ops.jsonl", opsRun],
] as const;

function load(raw: string): SequencedFrame[] {
  return raw
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line) as SequencedFrame);
}

function outline(raw: string) {
  const state = reduceFrames(initialTranscript, load(raw));
  const { container } = render(<TranscriptView items={visibleItems(state)} onAnswer={() => undefined} />);
  return [...container.querySelectorAll("[data-item]")].map((node) => node.getAttribute("data-item")).join("\n");
}

describe("chat transcript", () => {
  it("replays every chat_run fixture to a byte-stable outline", () => {
    const outlines = FIXTURES.map(([name, raw]) => `${name}\n${outline(raw)}`).join("\n---\n");
    expect(outlines).toBe(
      [
        "chat_run_decide.jsonl",
        "user:decide whether to continue",
        "turn:act:Running decide.|decide:no decision core is bound on this route",
        "input:No decision was recorded. Record a choice anyway?",
        "final:no decision core is bound on this route",
        "final:✓ Accept",
        "---",
        "chat_run_ops.jsonl",
        "user:fixture ops",
        "turn:act:|decide:no decision core is bound on this route|firewall eval:no firewall evaluation was recorded for this request|budget show:|verify run:no proof was recorded for this request|sandbox run:no sandbox run was recorded for this request",
        "final:Ran the fixture ops.",
      ].join("\n"),
    );
  });

  it("drops a frame and resumes without duplicating it", () => {
    const frames = load(decideRun);
    const dropped = frames.filter((frame) => frame.seq !== 8);
    let state = reduceFrames(initialTranscript, dropped);
    const replay = reduceTranscript(state, frames[1]!);
    expect(replay).toBe(state);
    expect(state.items.some((item) => item.kind === "turn" && item.ops.some((op) => op.output))).toBe(false);
    state = reduceTranscript(state, frames.find((frame) => frame.seq === 8)!);
    const ops = state.items.flatMap((item) => (item.kind === "turn" ? item.ops : []));
    expect(ops.filter((op) => op.id === "req-decide-op")).toHaveLength(1);
    expect(ops[0]?.output).toMatchObject({ reason: "no decision core is bound on this route" });
    const again = reduceTranscript(state, frames.find((frame) => frame.seq === 8)!);
    expect(again).toBe(state);
    const second = reduceFrames(state, [
      { seq: 1, frame: { type: "started", query: "budget show" } },
      { seq: 2, frame: { type: "run", requestId: "req-budget", sessionId: "ses-decide", query: "budget show" } },
    ]);
    expect(second.items.filter((item) => item.kind === "user")).toHaveLength(2);
    expect(second).not.toBe(state);
  });

  it("renders specialized op cards", () => {
    const state = reduceFrames(initialTranscript, load(opsRun));
    const { container } = render(<TranscriptView items={visibleItems(state)} onAnswer={() => undefined} />);
    expect(container.querySelector("[data-op-view='decide']")?.textContent).toContain("no decision core is bound on this route");
    expect(container.querySelector("[data-op-view='firewall']")?.textContent).toContain("no firewall evaluation was recorded");
    expect(container.querySelector("[data-op-view='budget']")?.textContent).toContain("meta_analysis");
    expect(container.querySelector("[data-op-view='verify']")?.textContent).toContain("no proof was recorded");
    expect(container.querySelector("[data-op-view='sandbox']")?.textContent).toContain("no sandbox run was recorded");
  });

  it("sends a button-grid answer", () => {
    const onAnswer = vi.fn();
    render(
      <ButtonGrid
        request={{ kind: "buttons", prompt: "Choose", options: [{ label: "Accept", value: "accept" }] }}
        onAnswer={onAnswer}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Accept" }));
    expect(onAnswer).toHaveBeenCalledWith("accept");
  });

  it("rejects a non-image and keeps the degradation note", () => {
    const onChange = vi.fn();
    const { container } = render(<ImageChooser images={[]} onChange={onChange} />);
    expect(container.querySelector("[data-image-degraded]")?.textContent).toContain("captions only");
    const input = container.querySelector("input[type='file']");
    expect(input).toBeTruthy();
    fireEvent.change(input!, { target: { files: [new File(["x"], "note.txt", { type: "text/plain" })] } });
    expect(screen.getByRole("alert").textContent).toContain("not an accepted image type");
    fireEvent.change(input!, { target: { files: [new File(["x"], "shot.png", { type: "image/png" })] } });
    expect(onChange).toHaveBeenCalledWith([expect.objectContaining({ name: "shot.png", type: "image/png" })]);
  });

  it("sends on ⌘↵", () => {
    const onSend = vi.fn();
    render(<Composer onSend={onSend} />);
    const box = screen.getByRole("textbox");
    fireEvent.change(box, { target: { value: "budget show" } });
    fireEvent.keyDown(box, { key: "Enter", metaKey: true });
    expect(onSend).toHaveBeenCalledWith("budget show", [], []);
  });

  it("virtualizes a long transcript", () => {
    const previous = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
      configurable: true,
      get() {
        return this instanceof HTMLElement && this.hasAttribute("data-transcript-scroll") ? 384 : 22;
      },
    });
    try {
      const frames: SequencedFrame[] = Array.from({ length: 1000 }, (_, index) => ({
        seq: index + 1,
        frame: { type: "run", requestId: `r${index}`, sessionId: "s", query: `line ${index}` } as ChatFrame,
      }));
      const state = reduceFrames(initialTranscript, frames);
      const { container } = render(<TranscriptView items={visibleItems(state)} onAnswer={() => undefined} />);
      expect(container.querySelectorAll("[data-item]").length).toBeLessThan(80);
    } finally {
      if (previous) Object.defineProperty(HTMLElement.prototype, "offsetHeight", previous);
    }
  });
});
