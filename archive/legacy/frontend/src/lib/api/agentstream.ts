import { fetchJson, withQuery } from "@/lib/api/client.ts";
import type {
  AgentList,
  AgentRunView,
  AgentstreamEvent,
  StatusValue,
  TraceEventPage,
} from "@/lib/wire/inspector.ts";

export function fetchAgentRuns(sessionId?: string): Promise<AgentList> {
  return fetchJson<AgentList>(
    withQuery("/api/agents", { session_id: sessionId }),
  );
}

export function fetchAgentRun(runId: string): Promise<AgentRunView> {
  return fetchJson<AgentRunView>(`/api/agents/${encodeURIComponent(runId)}`);
}

export function fetchAgentEvents(runId: string, from = 1): Promise<TraceEventPage> {
  return fetchJson<TraceEventPage>(
    withQuery(`/api/agents/${encodeURIComponent(runId)}/events`, { from }),
  );
}

export function fetchAgentstreamEvents(
  agentstreamId: string,
  from = 1,
): Promise<TraceEventPage> {
  return fetchJson<TraceEventPage>(
    withQuery(`/api/agentstream/sessions/${encodeURIComponent(agentstreamId)}/events`, { from }),
  );
}

export function agentstreamStreamUrl(agentstreamId: string, from?: number) {
  return withQuery(`/api/agentstream/sessions/${encodeURIComponent(agentstreamId)}/stream`, {
    from,
  });
}

export function cancelAgentRun(runId: string): Promise<AgentRunView> {
  return fetchJson(`/api/agents/${encodeURIComponent(runId)}/cancel`, { method: "POST" });
}

export function eventText(event: AgentstreamEvent): string {
  const payload = event.payload;
  if (payload === null || payload === undefined) return "";
  if (typeof payload === "string") return payload;
  if (typeof payload !== "object" || Array.isArray(payload)) return JSON.stringify(payload);
  const record = payload as Record<string, StatusValue>;
  for (const key of ["text", "message", "content", "summary", "detail", "label"]) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value;
  }
  return JSON.stringify(payload);
}

export function eventRole(event: AgentstreamEvent): string {
  const payload = event.payload;
  if (payload && typeof payload === "object" && !Array.isArray(payload)) {
    const role = (payload as Record<string, StatusValue>).role;
    if (typeof role === "string") return role;
  }
  switch (event.event_type) {
    case "user_message":
      return "user";
    case "agent_message":
      return "agent";
    case "tool_call":
    case "tool_result":
      return "tool";
    case "agent_turn_summary":
      return "summary";
    case "agent_status":
    case "lifecycle":
    case "presence_changed":
    case "status_updated":
      return "status";
    default:
      return "system";
  }
}
