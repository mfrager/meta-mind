import { applyDocFrame, type FocusFrame } from "@/lib/focus/apply.ts";
import type { FocusDoc } from "@/lib/wire/focus.ts";

export const FOCUS_FIXTURE: FocusDoc = {
  v: 1,
  id: "01jdocprose000000000000000",
  kind: "prose",
  title: "From chat",
  subtitle: "source chat",
  status: "saved",
  version: 2,
  source: { kind: "chat", refId: "fixture" },
  capabilities: ["edit", "export", "history", "provenance", "focus-chat"],
  snapshot: {
    markdown: "Opened from a focus frame.",
    production: true,
    path: "docs/console-note.md",
  },
};

export const FOCUS_FRAME: FocusFrame = { type: "focus", doc: FOCUS_FIXTURE };

export function openFocusFixture(open: FocusDoc[]) {
  return applyDocFrame(open, FOCUS_FRAME);
}
