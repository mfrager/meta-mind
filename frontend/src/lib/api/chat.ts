import type { ChatFrame } from "@/lib/wire/chat.ts";
import type { SequencedFrame } from "@/lib/transcript/reducer.ts";

async function readError(res: Response): Promise<Error> {
  const body = (await res.json().catch(() => null)) as { reason?: string; message?: string } | null;
  return new Error(body?.reason || body?.message || `HTTP ${res.status}`);
}

/**
 * Incremental SSE decoder.
 *
 * A network chunk can end anywhere — mid-frame, mid-line, even between the two
 * bytes of a multi-byte character — so the decoder keeps the incomplete tail and
 * only parses a block once its terminating blank line arrives.
 */
export class SseStreamParser {
  private buffer = "";

  push(chunk: string): SequencedFrame[] {
    this.buffer += chunk.replace(/\r\n/g, "\n");
    const frames: SequencedFrame[] = [];
    for (;;) {
      const boundary = this.buffer.indexOf("\n\n");
      if (boundary === -1) break;
      const block = this.buffer.slice(0, boundary);
      this.buffer = this.buffer.slice(boundary + 2);
      const parsed = parseSseBlock(block);
      if (parsed) frames.push(parsed);
    }
    return frames;
  }

  flush(): SequencedFrame[] {
    const rest = this.buffer;
    this.buffer = "";
    const parsed = rest.trim() ? parseSseBlock(rest) : null;
    return parsed ? [parsed] : [];
  }
}

function parseSseBlock(block: string): SequencedFrame | null {
  if (!block.trim() || block.startsWith(":")) return null;
  let seq = 0;
  let data = "";
  for (const line of block.split("\n")) {
    if (line.startsWith("id:")) seq = Number(line.slice(3).trim());
    if (line.startsWith("data:")) {
      const value = line.slice(5);
      data += value.startsWith(" ") ? value.slice(1) : value;
    }
  }
  if (!data) return null;
  try {
    return { seq, frame: JSON.parse(data) as ChatFrame };
  } catch {
    return null;
  }
}

export function parseSse(raw: string): SequencedFrame[] {
  const parser = new SseStreamParser();
  const frames = parser.push(raw);
  return [...frames, ...parser.flush()];
}

export async function createSession(): Promise<{ id: string; title: string; requestId?: string }> {
  const res = await fetch("/api/chat/sessions", { method: "POST" });
  if (!res.ok) throw await readError(res);
  return (await res.json()) as { id: string; title: string; requestId?: string };
}

export async function openChat(body: {
  sessionId?: string;
  query: string;
  refs?: string[];
  attachments?: { name: string; type: string; size: number; caption?: string }[];
  pane?: { id: string; title?: string };
  propose?: { name: string; input: unknown };
}): Promise<SequencedFrame[]> {
  const res = await fetch("/api/chat", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw await readError(res);
  return parseSse(await res.text());
}

/**
 * Open a chat turn and deliver every frame as soon as it arrives.
 *
 * Unlike [`openChat`], this never buffers the body: the first frames (run ids,
 * pass starts) reach the reducer while the model is still working. A server
 * that answers without a stream body is still handled, once, as a whole body.
 */
export async function openChatStream(
  body: Parameters<typeof openChat>[0],
  onFrame: (entry: SequencedFrame) => void,
  signal?: AbortSignal
): Promise<void> {
  const res = await fetch("/api/chat", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  if (!res.ok) throw await readError(res);
  if (!res.body) {
    for (const entry of parseSse(await res.text())) onFrame(entry);
    return;
  }
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  const parser = new SseStreamParser();
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      for (const entry of parser.push(decoder.decode(value, { stream: true }))) {
        onFrame(entry);
      }
    }
    for (const entry of parser.push(decoder.decode())) onFrame(entry);
    for (const entry of parser.flush()) onFrame(entry);
  } finally {
    reader.releaseLock();
  }
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

export async function answerRun(requestId: string, answer: string): Promise<SequencedFrame[]> {
  const res = await fetch(`/api/chat/${encodeURIComponent(requestId)}/answer`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ answer }),
  });
  if (!res.ok) throw await readError(res);
  const body = (await res.json()) as { frames: { seq: number; frame: ChatFrame }[] };
  return body.frames;
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
