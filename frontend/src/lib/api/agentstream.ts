import { fetchJson, withQuery } from "@/lib/api/client.ts";
import type { EventPage, HealthEnvelope, SessionList } from "@/lib/wire/agentstream.ts";

export const EVENT_PAGE_LIMIT = 500;

/** The session index, newest activity first. */
export function fetchSessions(): Promise<SessionList> {
  return fetchJson<SessionList>("/api/sessions");
}

/** One page of persisted records, straight from the JSONL. */
export function fetchEvents(sessionId: string, from = 1, limit = EVENT_PAGE_LIMIT): Promise<EventPage> {
  return fetchJson<EventPage>(
    withQuery(`/api/agentstream/sessions/${encodeURIComponent(sessionId)}/events`, { from, limit }),
  );
}

/** The SSE URL for a session cursor. The server replays from `from`, inclusive. */
export function streamUrl(sessionId: string, from: number): string {
  return withQuery(`/api/agentstream/sessions/${encodeURIComponent(sessionId)}/stream`, { from });
}

/** Minimal health envelope. */
export function fetchHealth(): Promise<HealthEnvelope> {
  return fetchJson<HealthEnvelope>("/api/health");
}
