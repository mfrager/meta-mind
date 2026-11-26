import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { agentstreamStreamUrl, fetchAgentstreamEvents } from "@/lib/api/agentstream.ts";
import type { AgentstreamEvent } from "@/lib/wire/inspector.ts";

const RECONNECT_MS = 1_500;

function documentVisible() {
  return typeof document === "undefined" || !document.hidden;
}

/** Load a trace and append live events only while its tab is visible. */
export function useTraceStream(agentstreamId: string | null, live: boolean) {
  const query = useQuery({
    queryKey: ["inspector", "trace", agentstreamId],
    queryFn: () => fetchAgentstreamEvents(agentstreamId as string),
    enabled: Boolean(agentstreamId),
  });
  const [events, setEvents] = useState<AgentstreamEvent[]>([]);
  const [visible, setVisible] = useState(documentVisible);
  const [connection, setConnection] = useState<"idle" | "live" | "reconnecting">("idle");
  const loadedFor = useRef<string | null>(null);
  const countRef = useRef(0);

  useEffect(() => {
    if (typeof document === "undefined") return;
    const onChange = () => setVisible(documentVisible());
    document.addEventListener("visibilitychange", onChange);
    return () => document.removeEventListener("visibilitychange", onChange);
  }, []);

  useEffect(() => {
    if (!query.data || loadedFor.current === agentstreamId) return;
    loadedFor.current = agentstreamId;
    setEvents(query.data.items);
  }, [agentstreamId, query.data]);

  useEffect(() => {
    loadedFor.current = null;
    setEvents([]);
    setConnection("idle");
  }, [agentstreamId]);

  useEffect(() => {
    countRef.current = events.length;
  }, [events.length]);

  useEffect(() => {
    if (!agentstreamId || !live || !visible) return;
    let cancelled = false;
    let source: EventSource | null = null;
    let retry: ReturnType<typeof setTimeout> | null = null;

    const connect = () => {
      if (cancelled) return;
      const next = new EventSource(agentstreamStreamUrl(agentstreamId, countRef.current + 1));
      source = next;
      next.onopen = () => {
        if (!cancelled) setConnection("live");
      };
      const append = (data: string) => {
        try {
          const event = JSON.parse(data) as AgentstreamEvent;
          if (!event?.event_id) return;
          setEvents((current) =>
            current.some((entry) => entry.event_id === event.event_id) ? current : [...current, event],
          );
        } catch {
          // Ignore a malformed frame; the next event still arrives.
        }
      };
      for (const type of [
        "user_message",
        "agent_message",
        "tool_call",
        "tool_result",
        "agent_turn",
        "agent_turn_summary",
        "agent_status",
        "status_updated",
        "proactive",
        "lifecycle",
      ]) {
        next.addEventListener(type, (event) => append((event as MessageEvent<string>).data));
      }
      next.onerror = () => {
        next.close();
        if (source === next) source = null;
        if (cancelled) return;
        setConnection("reconnecting");
        retry = setTimeout(connect, RECONNECT_MS);
      };
    };

    connect();
    return () => {
      cancelled = true;
      if (retry) clearTimeout(retry);
      source?.close();
    };
  }, [agentstreamId, live, visible]);

  return { ...query, events, connection };
}
