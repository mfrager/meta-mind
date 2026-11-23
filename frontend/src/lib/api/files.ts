import { fetchJson, withQuery } from "@/lib/api/client.ts";
import type { FileTreePage, LogPage } from "@/lib/wire/inspector.ts";

export function fetchFileTree(
  path: string,
  cursor?: string | null,
  limit = 50,
): Promise<FileTreePage> {
  return fetchJson<FileTreePage>(
    withQuery("/api/console/files/tree", { path, cursor, limit }),
  );
}

export function fetchLogs(
  sessionId: string,
  params?: {
    source?: string;
    mode?: string;
    cursor?: string | null;
    limit?: number;
    level?: string;
    q?: string;
  },
): Promise<LogPage> {
  return fetchJson<LogPage>(
    withQuery(`/api/console/sessions/${encodeURIComponent(sessionId)}/logs`, {
      source: params?.source,
      mode: params?.mode,
      cursor: params?.cursor,
      limit: params?.limit ?? 50,
      level: params?.level,
      q: params?.q,
    }),
  );
}
