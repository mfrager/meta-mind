import { describe, expect, it } from "vitest";

import { applyDocFrame, parseFocusEvent } from "@/lib/focus/apply.ts";
import { FOCUS_FIXTURE, FOCUS_FRAME } from "@/lib/focus/fixture.ts";
import type { FocusDoc } from "@/lib/wire/focus.ts";

const older: FocusDoc = { ...FOCUS_FIXTURE, version: 1, title: "Older" };

describe("applyDocFrame", () => {
  it("opens a document from a focus frame", () => {
    const next = applyDocFrame([], FOCUS_FRAME);
    expect(next.openedId).toBe(FOCUS_FIXTURE.id);
    expect(next.docs[0]?.title).toBe("From chat");
  });

  it("refreshes when the frame version is newer and ignores a stale frame", () => {
    const opened = applyDocFrame([], { type: "focus", doc: older });
    const refreshed = applyDocFrame(opened.docs, FOCUS_FRAME);
    expect(refreshed.docs[0]?.title).toBe("From chat");
    expect(refreshed.docs[0]?.version).toBe(2);
    const stale = applyDocFrame(refreshed.docs, { type: "focus", doc: older });
    expect(stale.docs[0]?.version).toBe(2);
    expect(stale.docs).toBe(refreshed.docs);
  });

  it("parses a focus-channel payload and drops anything else", () => {
    const frame = parseFocusEvent(JSON.stringify(FOCUS_FRAME));
    expect(frame?.doc.title).toBe("From chat");
    expect(parseFocusEvent('{"type":"status"}')).toBeNull();
    expect(parseFocusEvent("not-json")).toBeNull();
  });
});
