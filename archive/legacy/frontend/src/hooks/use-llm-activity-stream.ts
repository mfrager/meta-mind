import { useEffect, useState } from "react";

import { fetchLlmActivity, llmActivityStreamUrl } from "@/lib/api/llm-activity.ts";
import { useLlmLogStore } from "@/lib/store/llm-log.ts";
import type { LlmActivityEvent, LlmActivityResync } from "@/lib/wire/llm_activity.ts";

export const RECONNECT_MS = 1_800;

function documentVisible() {
  return typeof document === "undefined" || !document.hidden;
}

function messageOf(error: unknown) {
  return error instanceof Error ? error.message : "The LLM activity feed is unavailable.";
}

function parse<T>(data: string): T | null {
  try {
    return JSON.parse(data) as T;
  } catch {
    return null;
  }
}

/**
 * Hydrate the retained ring once, then follow the live tail. Mount this once at
 * workspace level so a pane remount neither re-fetches nor duplicates events.
 */
export function useLlmActivityStream() {
  const [visible, setVisible] = useState(documentVisible);

  useEffect(() => {
    if (typeof document === "undefined") return;
    const onChange = () => setVisible(documentVisible());
    document.addEventListener("visibilitychange", onChange);
    return () => document.removeEventListener("visibilitychange", onChange);
  }, []);

  useEffect(() => {
    if (!visible) {
      useLlmLogStore.getState().setConnection("idle");
      return;
    }

    let closed = false;
    let recovering = false;
    let source: EventSource | null = null;
    let retry: ReturnType<typeof setTimeout> | null = null;
    let frame: number | null = null;
    const pending: LlmActivityEvent[] = [];

    const setConnection = useLlmLogStore.getState().setConnection;

    const flush = () => {
      frame = null;
      const events = pending.splice(0, pending.length);
      for (const event of events) {
        if (closed) return;
        const state = useLlmLogStore.getState();
        if (event.seq <= state.lastSeq) continue;
        if (event.seq > state.lastSeq + 1 && state.lastSeq > 0) {
          void recover();
          return;
        }
        state.ingest(event.seq, event);
      }
    };

    const schedule = () => {
      if (frame !== null) return;
      frame =
        typeof requestAnimationFrame === "function"
          ? requestAnimationFrame(flush)
          : (setTimeout(flush, 16) as unknown as number);
    };

    const push = (event: LlmActivityEvent) => {
      if (recovering) return;
      pending.push(event);
      schedule();
    };

    const connect = () => {
      if (closed) return;
      source = new EventSource(llmActivityStreamUrl(useLlmLogStore.getState().lastSeq + 1));
      const current = source;
      current.onopen = () => {
        if (!closed) setConnection("live");
      };
      current.addEventListener("activity", (message) => {
        const event = parse<LlmActivityEvent>((message as MessageEvent<string>).data);
        if (event && typeof event.seq === "number") push(event);
      });
      current.addEventListener("resync", (message) => {
        const snapshot = parse<LlmActivityResync>((message as MessageEvent<string>).data);
        if (!snapshot || !Array.isArray(snapshot.events)) return;
        useLlmLogStore.getState().resyncEvents(snapshot);
      });
      current.onerror = () => {
        if (closed) return;
        current.close();
        if (source === current) source = null;
        setConnection("reconnecting");
        if (retry !== null) clearTimeout(retry);
        retry = setTimeout(() => {
          retry = null;
          connect();
        }, RECONNECT_MS);
      };
    };

    const recover = async () => {
      if (recovering || closed) return;
      recovering = true;
      source?.close();
      source = null;
      if (retry !== null) {
        clearTimeout(retry);
        retry = null;
      }
      try {
        const snapshot = await fetchLlmActivity({ limit: 200 });
        if (closed) return;
        useLlmLogStore.getState().resync(snapshot);
        setConnection("connecting");
        recovering = false;
        connect();
      } catch (error) {
        recovering = false;
        if (!closed) setConnection("error", messageOf(error));
      }
    };

    const start = async () => {
      if (useLlmLogStore.getState().hydrated) {
        connect();
        return;
      }
      setConnection("loading");
      try {
        const snapshot = await fetchLlmActivity({ limit: 200 });
        if (closed) return;
        useLlmLogStore.getState().resync(snapshot);
        setConnection("connecting");
        connect();
      } catch (error) {
        if (!closed) setConnection("error", messageOf(error));
      }
    };

    void start();

    return () => {
      closed = true;
      source?.close();
      source = null;
      if (retry !== null) clearTimeout(retry);
      if (frame !== null) {
        if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(frame);
        else clearTimeout(frame);
      }
    };
  }, [visible]);
}
