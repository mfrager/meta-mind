import { useEffect } from "react";

import { EVENT_PAGE_LIMIT, fetchEvents, streamUrl } from "@/lib/api/agentstream.ts";
import { EVENT_TYPES } from "@/lib/transcript/model.ts";
import { useAgentstreamStore } from "@/lib/store/agentstream.ts";
import type { Event, EventPage } from "@/lib/wire/agentstream.ts";

const RECONNECT_BASE_MS = 250;
const RECONNECT_MAX_MS = 8_000;

/**
 * Keep one session in the store: snapshot from the JSONL, then tail the SSE
 * stream from `lastSeq + 1`. A stream gap or a `resync` frame refills from the
 * last applied cursor, and the store drops any record at or below it, so a
 * reconnect inside a turn duplicates nothing.
 */
export function useAgentstream(sessionId: string | null, enabled = true) {
  useEffect(() => {
    const store = useAgentstreamStore.getState();
    store.begin(sessionId);
    if (!sessionId || !enabled) return;

    let disposed = false;
    let source: EventSource | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let retry = 0;
    let catchingUp = false;

    const ingest = (events: Event[]) => {
      if (!disposed) useAgentstreamStore.getState().ingest(events);
    };

    const snapshot = async (from: number): Promise<void> => {
      let cursor = Math.max(1, from);
      for (let page = 0; page < 100; page += 1) {
        const result = await fetchEvents(sessionId, cursor, EVENT_PAGE_LIMIT);
        if (disposed) return;
        ingest(result.events);
        if (result.events.length === 0) return;
        if (result.events.length < EVENT_PAGE_LIMIT) return;
        if (result.next <= cursor) return;
        cursor = result.next;
      }
    };

    const close = () => {
      source?.close();
      source = null;
    };

    const open = () => {
      if (disposed) return;
      const state = useAgentstreamStore.getState();
      state.setConnection(retry > 0 ? "reconnecting" : "connecting");
      const next = new EventSource(streamUrl(sessionId, state.lastSeq + 1));
      source = next;

      next.onopen = () => {
        retry = 0;
        useAgentstreamStore.getState().setConnection("live");
      };

      const onEvent = (message: MessageEvent<string>) => {
        let event: Event;
        try {
          event = JSON.parse(message.data) as Event;
        } catch {
          return;
        }
        if (event.type === "llm.delta") {
          ingest([event]);
          return;
        }
        const expected = useAgentstreamStore.getState().lastSeq + 1;
        if (event.seq > expected) {
          void catchUp();
          return;
        }
        ingest([event]);
      };

      const resync = (message: MessageEvent<string>) => {
        try {
          const page = JSON.parse(message.data) as EventPage;
          ingest(page.events);
        } catch {
          // A malformed resync frame is left to the next reconnect.
        }
      };

      for (const type of EVENT_TYPES) {
        next.addEventListener(type, onEvent as EventListener);
      }
      next.addEventListener("resync", resync as EventListener);

      next.onerror = () => {
        close();
        if (disposed) return;
        useAgentstreamStore.getState().setConnection("reconnecting");
        retry += 1;
        const delay = Math.min(RECONNECT_MAX_MS, RECONNECT_BASE_MS * 2 ** Math.min(retry, 5));
        timer = setTimeout(() => {
          if (!disposed) open();
        }, delay);
      };
    };

    const catchUp = async () => {
      if (catchingUp || disposed) return;
      catchingUp = true;
      close();
      try {
        await snapshot(useAgentstreamStore.getState().lastSeq + 1);
      } catch {
        // Reconnect handles a failed refill; the cursor is unchanged.
      } finally {
        catchingUp = false;
      }
      open();
    };

    void snapshot(1)
      .then(() => {
        if (!disposed) open();
      })
      .catch((error: unknown) => {
        if (disposed) return;
        useAgentstreamStore
          .getState()
          .setConnection("error", error instanceof Error ? error.message : "The event stream could not load.");
      });

    return () => {
      disposed = true;
      close();
      if (timer) clearTimeout(timer);
    };
  }, [sessionId, enabled]);
}
