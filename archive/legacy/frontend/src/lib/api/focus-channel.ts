import { useEffect } from "react";

import { focusStreamUrl } from "@/lib/api/focus.ts";
import { parseFocusEvent } from "@/lib/focus/apply.ts";
import { recordStageTick } from "@/lib/live/stages.ts";
import { useDocuments } from "@/lib/store/documents.ts";

/** Subscribe to the focus channel for a live document and apply newer frames. */
export function useFocusChannel(docId: string | null, from: number) {
  const applyFrame = useDocuments((state) => state.applyFrame);
  useEffect(() => {
    if (!docId || typeof EventSource === "undefined") return;
    const source = new EventSource(focusStreamUrl(docId, from));
    const onFocus = (event: Event) => {
      const data = (event as MessageEvent<string>).data;
      const frame = typeof data === "string" ? parseFocusEvent(data) : null;
      if (frame) applyFrame(frame);
    };
    source.addEventListener("focus", onFocus);
    const onTick = (event: Event) => {
      const data = (event as MessageEvent<string>).data;
      if (typeof data !== "string") return;
      try {
        const tick = JSON.parse(data) as { docId?: string; reason?: string };
        if (tick.docId && tick.reason) recordStageTick({ docId: tick.docId, reason: tick.reason });
      } catch {
        // A tick the client cannot read stays out of the ticker.
      }
    };
    source.addEventListener("stage_tick", onTick);
    return () => {
      source.removeEventListener("focus", onFocus);
      source.removeEventListener("stage_tick", onTick);
      source.close();
    };
  }, [applyFrame, docId, from]);
}
