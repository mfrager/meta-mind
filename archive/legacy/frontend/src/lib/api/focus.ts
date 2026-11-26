import type { FocusDoc } from "@/lib/wire/focus.ts";

export type Provenance = {
  events: number;
  traceId: string | null;
  createdBy: string;
  updatedAt: string | null;
};

export type HistoryEntry = {
  version: number;
  savedAt: string;
  actor: string;
  status: string;
};

export type ChangesetCreated = {
  id: string;
  status: string;
  doc: FocusDoc | null;
};

async function readError(res: Response): Promise<Error> {
  const body = (await res.json().catch(() => null)) as { reason?: string; message?: string } | null;
  const reason = body?.reason || body?.message || `HTTP ${res.status}`;
  return new Error(reason);
}

export async function fetchDoc(id: string): Promise<FocusDoc> {
  const res = await fetch(`/api/focus/${encodeURIComponent(id)}`);
  if (!res.ok) throw await readError(res);
  return (await res.json()) as FocusDoc;
}

export async function saveDraft(id: string, body: string): Promise<FocusDoc> {
  const res = await fetch(`/api/focus/${encodeURIComponent(id)}/draft`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ body }),
  });
  if (!res.ok) throw await readError(res);
  return (await res.json()) as FocusDoc;
}

export async function fetchHistory(id: string): Promise<HistoryEntry[]> {
  const res = await fetch(`/api/focus/${encodeURIComponent(id)}/history`);
  if (!res.ok) throw await readError(res);
  return (await res.json()) as HistoryEntry[];
}

export async function fetchProvenance(id: string): Promise<Provenance> {
  const res = await fetch(`/api/focus/${encodeURIComponent(id)}/provenance`);
  if (!res.ok) throw await readError(res);
  return (await res.json()) as Provenance;
}

export async function createChangeset(input: {
  docId: string;
  reason: string;
  summary: string;
  path: string;
  content: string;
}): Promise<ChangesetCreated> {
  const res = await fetch("/api/changesets", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });
  if (!res.ok) throw await readError(res);
  return (await res.json()) as ChangesetCreated;
}

export function focusStreamUrl(sessionId: string, from: number) {
  return `/api/focus/${encodeURIComponent(sessionId)}/stream?from=${from}`;
}
