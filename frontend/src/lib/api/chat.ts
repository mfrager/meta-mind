import type { ChatFrame } from "@/lib/wire/chat.ts";
import type { SequencedFrame } from "@/lib/transcript/reducer.ts";

async function readError(res: Response): Promise<Error> {
  const body = (await res.json().catch(() => null)) as { reason?: string; message?: string } | null;
  return new Error(body?.reason || body?.message || `HTTP ${res.status}`);
}

export function parseSse(raw: string): SequencedFrame[] {
  const frames: SequencedFrame[] = [];
  for (const chunk of raw.split("\n\n")) {
    if (!chunk.trim() || chunk.startsWith(":")) continue;
    let seq = 0;
    let data = "";
    for (const line of chunk.split("\n")) {
      if (line.startsWith("id: ")) seq = Number(line.slice(4));
      if (line.startsWith("data: ")) data += line.slice(6);
    }
    if (!data) continue;
    frames.push({ seq, frame: JSON.parse(data) as ChatFrame });
  }
  return frames;
}

export async function createSession(): Promise<{ id: string; title: string }> {
  const res = await fetch("/api/chat/sessions", { method: "POST" });
  if (!res.ok) throw await readError(res);
  return (await res.json()) as { id: string; title: string };
}

export async function openChat(body: {
  sessionId?: string;
  query: string;
  refs?: string[];
  attachments?: { name: string; type: string; size: number; caption?: string }[];
}): Promise<SequencedFrame[]> {
  const res = await fetch("/api/chat", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw await readError(res);
  return parseSse(await res.text());
}

export async function resumeChat(requestId: string, from: number): Promise<SequencedFrame[]> {
  const res = await fetch(`/api/chat/${encodeURIComponent(requestId)}/stream?from=${from}`);
  if (!res.ok) throw await readError(res);
  return parseSse(await res.text());
}

export async function stopChat(requestId: string): Promise<{ state: string; reason?: string }> {
  const res = await fetch(`/api/chat/${encodeURIComponent(requestId)}/stop`, { method: "POST" });
  if (!res.ok) throw await readError(res);
  return (await res.json()) as { state: string; reason?: string };
}

export async function regenerateChat(requestId: string): Promise<SequencedFrame[]> {
  const res = await fetch(`/api/chat/${encodeURIComponent(requestId)}/regenerate`, { method: "POST" });
  if (!res.ok) throw await readError(res);
  return parseSse(await res.text());
}

export async function branchChat(sessionId: string): Promise<{ sessionId: string; requestId: string }> {
  const res = await fetch(`/api/chat/sessions/${encodeURIComponent(sessionId)}/branch`, { method: "POST" });
  if (!res.ok) throw await readError(res);
  return (await res.json()) as { sessionId: string; requestId: string };
}

export async function answerInput(sessionId: string, id: string, answer: string): Promise<SequencedFrame[]> {
  const res = await fetch(`/api/chat/sessions/${encodeURIComponent(sessionId)}/input-answer`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id, answer }),
  });
  if (!res.ok) throw await readError(res);
  const body = (await res.json()) as { frames: { seq: number; frame: ChatFrame }[] };
  return body.frames;
}
