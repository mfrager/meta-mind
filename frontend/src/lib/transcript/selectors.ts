import type { TranscriptItem, TranscriptState } from "@/lib/transcript/reducer.ts";

/** Items the log should show. Hidden turns stay in state so a later frame can reveal them. */
export function visibleItems(state: TranscriptState): TranscriptItem[] {
  return state.items.filter((item) => item.kind !== "turn" || item.visible);
}

export function suggestionsOf(state: TranscriptState): string[] {
  const finals = state.items.filter((item) => item.kind === "final");
  const last = finals.at(-1);
  return last && last.kind === "final" ? last.suggestions : [];
}
