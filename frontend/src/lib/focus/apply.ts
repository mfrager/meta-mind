import type { FocusDoc } from "@/lib/wire/focus.ts";

export type FocusFrame = { type: "focus"; doc: FocusDoc };

/** Parse one SSE `focus` payload. Other frame types are ignored. */
export function parseFocusEvent(data: string): FocusFrame | null {
  let parsed: { type?: string; doc?: FocusDoc };
  try {
    parsed = JSON.parse(data) as { type?: string; doc?: FocusDoc };
  } catch {
    return null;
  }
  if (parsed.type !== "focus" || !parsed.doc || typeof parsed.doc.id !== "string") return null;
  return { type: "focus", doc: parsed.doc };
}

/** Apply one chat `focus` frame. Older versions are ignored. */
export function applyDocFrame(open: FocusDoc[], frame: FocusFrame): { docs: FocusDoc[]; openedId: string } {
  const doc = frame.doc;
  const existing = open.find((item) => item.id === doc.id);
  if (existing && doc.version < existing.version) {
    return { docs: open, openedId: existing.id };
  }
  const docs = existing ? open.map((item) => (item.id === doc.id ? doc : item)) : [...open, doc];
  return { docs, openedId: doc.id };
}
