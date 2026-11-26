import { afterEach, describe, expect, it, vi } from "vitest";

import { SseStreamParser, openChatStream, parseSse } from "@/lib/api/chat";

function block(seq: number, frame: unknown): string {
  return `id: ${seq}\nevent: ${(frame as { type: string }).type}\ndata: ${JSON.stringify(frame)}\n\n`;
}

describe("SseStreamParser", () => {
  it("decodes frames split across arbitrary chunk boundaries", () => {
    const raw =
      block(1, { type: "started", query: "hi" }) +
      block(2, { type: "pass_start", pass: "process", turn_id: "t", attempt: 1 });
    const parser = new SseStreamParser();
    const frames = [
      ...parser.push(raw.slice(0, 7)),
      ...parser.push(raw.slice(7, 41)),
      ...parser.push(raw.slice(41)),
      ...parser.flush(),
    ];
    expect(frames.map((entry) => entry.seq)).toEqual([1, 2]);
    expect(frames[1].frame).toMatchObject({ type: "pass_start", pass: "process" });
  });

  it("keeps a frame whose blank line has not arrived yet", () => {
    const parser = new SseStreamParser();
    expect(
      parser.push(`id: 4\ndata: {"type":"completed","subsystems":[]}`)
    ).toEqual([]);
    expect(parser.flush()).toEqual([
      { seq: 4, frame: { type: "completed", subsystems: [] } },
    ]);
  });

  it("ignores comments and malformed payloads instead of throwing", () => {
    expect(parseSse(": keep-alive\n\nid: 1\ndata: not-json\n\n")).toEqual([]);
  });
});

describe("openChatStream", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("settles on the terminal frame even when the body stays open", async () => {
    const encoder = new TextEncoder();
    const reader = {
      read: vi
        .fn()
        .mockResolvedValueOnce({
          done: false,
          value: encoder.encode(
            block(1, { type: "final_response", text: "hi" }) +
              block(2, { type: "completed", subsystems: [] })
          ),
        })
        // A body that never closes: only the terminal frame can end the turn.
        .mockImplementation(() => new Promise(() => {})),
      cancel: vi.fn(async () => {}),
      releaseLock: vi.fn(),
    };
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({ ok: true, body: { getReader: () => reader } }))
    );

    const seen: number[] = [];
    await openChatStream({ query: "hello" }, (entry) => seen.push(entry.seq));

    expect(seen).toEqual([1, 2]);
    expect(reader.cancel).toHaveBeenCalledOnce();
    expect(reader.read).toHaveBeenCalledOnce();
  });
});
