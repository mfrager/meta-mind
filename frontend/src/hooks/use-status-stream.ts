import { useEffect, useRef, useState } from "react";

import { fetchStatus, statusStreamUrl } from "@/lib/api/inspector.ts";
import { sendPresence } from "@/lib/api/sessions.ts";
import { useSessionStatus, useStatusStore } from "@/lib/store/status.ts";
import type { StatusFrame } from "@/lib/wire/inspector.ts";

const STALE_AFTER_MS = 20_000;
const PRESENCE_HEARTBEAT_MS = 20_000;
const WATCHDOG_INTERVAL_MS = 5_000;
const RECONNECT_MS = 1_800;

function documentVisible() {
  return typeof document === "undefined" || !document.hidden;
}

/**
 * One status subscription per session.
 *
 * The hook fetches a full snapshot before opening SSE, pauses while the tab is
 * hidden, resumes with `from=<revision>`, and applies at most one frame per
 * animation frame. A frame older than the current revision is ignored by the
 * reducer, so reconnect replays and duplicate frames cannot change the DOM.
 */
export function useStatusStream(sessionId: string | null, enabled = true) {
  const state = useSessionStatus(sessionId);
  const [visible, setVisible] = useState(documentVisible);
  const pending = useRef<StatusFrame | null>(null);
  const raf = useRef<number | null>(null);
  const refreshing = useRef(false);

  const hasView = Boolean(state.view);

  useEffect(() => {
    if (typeof document === "undefined") return;
    const onChange = () => setVisible(documentVisible());
    document.addEventListener("visibilitychange", onChange);
    return () => document.removeEventListener("visibilitychange", onChange);
  }, []);

  useEffect(() => {
    if (!sessionId || !enabled) return;
    const store = useStatusStore.getState();
    store.ensure(sessionId);
    if (store.bySession[sessionId]?.view) return;
    const controller = new AbortController();
    store.begin(sessionId);
    void fetchStatus(sessionId)
      .then((view) => {
        if (controller.signal.aborted) return;
        useStatusStore.getState().setView(sessionId, view);
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        useStatusStore
          .getState()
          .setConnection(
            sessionId,
            "error",
            error instanceof Error ? error.message : "status request failed",
          );
      });
    return () => controller.abort();
  }, [sessionId, enabled]);

  useEffect(() => {
    if (!sessionId || !enabled || !visible || !hasView) return;
    let cancelled = false;
    let source: EventSource | null = null;
    let retry: ReturnType<typeof setTimeout> | null = null;

    const flush = () => {
      raf.current = null;
      const frame = pending.current;
      pending.current = null;
      if (!frame || cancelled) return;
      useStatusStore.getState().setView(sessionId, frame);
    };

    const enqueue = (frame: StatusFrame) => {
      if (!pending.current || frame.revision > pending.current.revision) pending.current = frame;
      if (raf.current !== null) return;
      raf.current =
        typeof requestAnimationFrame === "function"
          ? requestAnimationFrame(flush)
          : (setTimeout(flush, 0) as unknown as number);
    };

    const connect = () => {
      if (cancelled) return;
      const revision = useStatusStore.getState().bySession[sessionId]?.view?.revision;
      const next = new EventSource(statusStreamUrl(sessionId, revision));
      source = next;
      next.onopen = () => {
        if (!cancelled) useStatusStore.getState().setConnection(sessionId, "live");
      };
      next.addEventListener("status", (event) => {
        try {
          const frame = JSON.parse((event as MessageEvent<string>).data) as StatusFrame;
          if (typeof frame.revision === "number") enqueue(frame);
        } catch {
          // A malformed frame is ignored; the next revision still arrives.
        }
      });
      next.addEventListener("heartbeat", () => {
        if (!cancelled) useStatusStore.getState().setConnection(sessionId, "live");
      });
      next.onerror = () => {
        next.close();
        if (source === next) source = null;
        if (cancelled) return;
        useStatusStore.getState().setConnection(sessionId, "reconnecting");
        retry = setTimeout(connect, RECONNECT_MS);
      };
    };

    connect();

    return () => {
      cancelled = true;
      if (retry) clearTimeout(retry);
      source?.close();
      if (raf.current !== null) {
        if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(raf.current);
        else clearTimeout(raf.current);
      }
      raf.current = null;
      pending.current = null;
    };
  }, [sessionId, enabled, visible, hasView]);

  useEffect(() => {
    if (!sessionId || !enabled || !visible) return;
    const ping = () => {
      void sendPresence(sessionId).catch(() => undefined);
    };
    ping();
    const timer = setInterval(ping, PRESENCE_HEARTBEAT_MS);
    return () => clearInterval(timer);
  }, [sessionId, enabled, visible]);

  useEffect(() => {
    if (!sessionId || !enabled || !visible) return;
    const timer = setInterval(() => {
      const current = useStatusStore.getState().bySession[sessionId];
      if (!current || refreshing.current) return;
      if (Date.now() - current.lastFrameAt < STALE_AFTER_MS) return;
      refreshing.current = true;
      void fetchStatus(sessionId)
        .then((view) => {
          useStatusStore.getState().setView(sessionId, view);
          useStatusStore.getState().touch(sessionId);
        })
        .catch(() => useStatusStore.getState().markStale(sessionId))
        .finally(() => {
          refreshing.current = false;
        });
    }, WATCHDOG_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [sessionId, enabled, visible]);

  return state;
}

