import { ApiError, fetchJson, withQuery } from "@/lib/api/client.ts";
import type {
  ConversationStateView,
  SessionPage,
  SessionTrace,
  TracePage,
} from "@/lib/wire/inspector.ts";

export type SessionListParams = {
  cursor?: string | null;
  limit?: number;
  status?: string;
  q?: string;
  userId?: string;
};

export function fetchSessionPage(params: SessionListParams): Promise<SessionPage> {
  return fetchJson<SessionPage>(
    withQuery("/api/console/sessions", {
      cursor: params.cursor,
      limit: params.limit ?? 20,
      status: params.status,
      q: params.q,
      userId: params.userId,
    }),
  );
}

export function createConsoleSession(input?: {
  resumeSessionId?: string;
  previousSessionId?: string;
  userId?: string;
}): Promise<{ sessionId: string }> {
  return fetchJson("/api/console/sessions", {
    method: "POST",
    body: JSON.stringify({
      resumeSessionId: input?.resumeSessionId,
      previousSessionId: input?.previousSessionId,
      userId: input?.userId,
    }),
  });
}

export function resumeSession(sessionId: string): Promise<{ sessionId: string }> {
  return fetchJson(`/api/console/sessions/${encodeURIComponent(sessionId)}/resume`, {
    method: "POST",
  });
}

export function sendPresence(sessionId: string): Promise<{ presence: string; at: number }> {
  return fetchJson(`/api/console/sessions/${encodeURIComponent(sessionId)}/presence`, {
    method: "POST",
  });
}

export function endSession(sessionId: string): Promise<unknown> {
  return fetchJson(`/api/console/sessions/${encodeURIComponent(sessionId)}/end`, {
    method: "POST",
  });
}

export async function deleteConsoleSession(
  sessionId: string,
  mode: "soft" | "purge",
  lifecycle?: string,
): Promise<{ deleted: boolean }> {
  if (lifecycle === "active") await endSession(sessionId);
  try {
    return await fetchJson(
      withQuery(`/api/console/sessions/${encodeURIComponent(sessionId)}`, { mode }),
      { method: "DELETE" },
    );
  } catch (error) {
    // The rows in the Files tree do not always carry lifecycle metadata. If
    // the server refuses an active session, end it and retry once.
    if (!(error instanceof ApiError) || error.status !== 409) throw error;
    await endSession(sessionId);
    return fetchJson(
      withQuery(`/api/console/sessions/${encodeURIComponent(sessionId)}`, { mode }),
      { method: "DELETE" },
    );
  }
}

export function fetchConversation(sessionId: string): Promise<ConversationStateView> {
  return fetchJson<ConversationStateView>(
    `/api/console/sessions/${encodeURIComponent(sessionId)}/state`,
  );
}

export function fetchTraces(
  sessionId: string,
  params?: { cursor?: string | null; limit?: number },
): Promise<TracePage> {
  return fetchJson<TracePage>(
    withQuery(`/api/console/sessions/${encodeURIComponent(sessionId)}/traces`, {
      cursor: params?.cursor,
      limit: params?.limit ?? 20,
    }),
  );
}

export type { SessionTrace };
