import type { TreeOp, TreeSnapshot } from "@/lib/trees/model.ts";

async function readError(res: Response): Promise<Error> {
  const body = (await res.json().catch(() => null)) as { reason?: string; message?: string } | null;
  return new Error(body?.reason || body?.message || `HTTP ${res.status}`);
}

export async function fetchTree(name: string): Promise<TreeSnapshot> {
  const res = await fetch(`/api/trees/${encodeURIComponent(name)}`);
  if (!res.ok) throw await readError(res);
  return (await res.json()) as TreeSnapshot;
}

export async function applyTreeOps(name: string, ops: TreeOp[]): Promise<{ eventId: string; tree: TreeSnapshot }> {
  const res = await fetch(`/api/trees/${encodeURIComponent(name)}/ops`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ ops }),
  });
  if (!res.ok) throw await readError(res);
  return (await res.json()) as { eventId: string; tree: TreeSnapshot };
}

export async function fetchMap(name: string): Promise<TreeSnapshot & { rootId: string }> {
  const res = await fetch(`/api/maps/${encodeURIComponent(name)}`);
  if (!res.ok) throw await readError(res);
  return (await res.json()) as TreeSnapshot & { rootId: string };
}
