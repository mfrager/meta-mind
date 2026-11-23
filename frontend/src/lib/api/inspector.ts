import { fetchJson, withQuery } from "@/lib/api/client.ts";
import type {
  ActionList,
  AutomationInfo,
  DataHealth,
  PlanDetail,
  PlanList,
  ProcessList,
  StatusView,
  StateBoardPayload,
} from "@/lib/wire/inspector.ts";

export function fetchStatus(sessionId: string): Promise<StatusView> {
  return fetchJson<StatusView>(`/api/console/sessions/${encodeURIComponent(sessionId)}/status`);
}

export function statusStreamUrl(sessionId: string, from?: number) {
  return withQuery(`/api/console/sessions/${encodeURIComponent(sessionId)}/status/stream`, {
    from,
  });
}

export function fetchDataHealth(): Promise<DataHealth> {
  return fetchJson<DataHealth>("/api/console/data/health");
}

export function fetchPlans(): Promise<PlanList> {
  return fetchJson<PlanList>("/api/console/plans");
}

export function fetchPlan(planId: string): Promise<PlanDetail> {
  return fetchJson<PlanDetail>(`/api/console/plans/${planId}`);
}

export function fetchProcesses(sessionId?: string): Promise<ProcessList> {
  return fetchJson<ProcessList>(withQuery("/api/cme/processes", { session_id: sessionId }));
}

export function controlProcess(
  processId: string,
  action: "pause" | "resume" | "cancel" | "retry",
  body: { input?: string; step_id?: string } = {},
): Promise<{ process: unknown }> {
  return fetchJson(`/api/cme/processes/${encodeURIComponent(processId)}/${action}`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export function fetchActions(sessionId?: string): Promise<ActionList> {
  return fetchJson<ActionList>(withQuery("/api/console/actions", { sessionId }));
}

export function cancelAction(actionId: string): Promise<{ actionId: string; cancelled: boolean }> {
  return fetchJson(`/api/console/actions/${actionId}/cancel`, { method: "POST" });
}

export function fetchAutomation(): Promise<AutomationInfo> {
  return fetchJson<AutomationInfo>("/api/console/automation");
}

export function fetchStateBoard(): Promise<StateBoardPayload> {
  return fetchJson<StateBoardPayload>("/api/state");
}
