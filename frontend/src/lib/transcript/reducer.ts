import type { Event } from "@/lib/wire/agentstream.ts";
import { callIdOf, dataOf, num, str, usageOf } from "@/lib/transcript/model.ts";

export type PassStatus = "running" | "completed" | "failed" | "retried";

export type PassView = {
  pass: string;
  attempt: number;
  model: string | null;
  status: PassStatus;
  text: string;
  streaming: boolean;
  durationMs?: number;
  tokensIn?: number;
  tokensOut?: number;
  retryReason?: string;
};

export type TranscriptItem =
  | { kind: "user"; id: string; text: string }
  | { kind: "passes"; id: string; passes: PassView[] }
  | { kind: "assistant"; id: string; text: string; verified: boolean }
  | { kind: "error"; id: string; message: string; detail?: string };

/**
 * Project persisted records plus live deltas into the transcript.
 *
 * A duplicate `seq` is ignored, so replaying a page or reconnecting cannot
 * duplicate an item. Passes stay in the order they started; the assistant item
 * appears only when the pipeline writes `agent.message` (or a completed turn
 * carries a reply), never before the call succeeds.
 */
export function deriveTranscript(records: Event[], deltas: Record<string, string> = {}): TranscriptItem[] {
  const seen = new Set<number>();
  const items: TranscriptItem[] = [];
  const passesByTurn = new Map<string, PassView[]>();
  const callPass = new Map<string, PassView>();

  const upsertPass = (turnId: string, pass: string, attempt: number): PassView => {
    let list = passesByTurn.get(turnId);
    if (!list) {
      list = [];
      passesByTurn.set(turnId, list);
      items.push({ kind: "passes", id: turnId, passes: list });
    }
    let view = list.find((entry) => entry.pass === pass && entry.attempt === attempt);
    if (!view) {
      view = { pass, attempt, model: null, status: "running", text: "", streaming: false };
      list.push(view);
    }
    return view;
  };

  for (const event of records) {
    if (event.seq > 0) {
      if (seen.has(event.seq)) continue;
      seen.add(event.seq);
    }
    const data = dataOf(event);
    const turnId = event.turnId ?? "session";

    switch (event.type) {
      case "turn.started": {
        const text = str(data.userInput) ?? str(data.text) ?? "";
        if (!items.some((item) => item.kind === "user" && item.id === turnId)) {
          items.push({ kind: "user", id: turnId, text });
        }
        break;
      }
      case "user.message": {
        const text = str(data.text) ?? "";
        if (!items.some((item) => item.kind === "user" && item.id === turnId)) {
          items.push({ kind: "user", id: turnId, text });
        }
        break;
      }
      case "pass.started": {
        const pass = str(data.pass) ?? "pass";
        const view = upsertPass(turnId, pass, num(data.attempt) ?? 1);
        view.model = event.model ?? str(data.model) ?? view.model;
        view.status = "running";
        break;
      }
      case "llm.request": {
        const pass = str(data.pass) ?? "call";
        const view = upsertPass(turnId, pass, num(data.attempt) ?? 1);
        view.model = event.model ?? str(data.model) ?? view.model;
        view.status = "running";
        const callId = callIdOf(event);
        if (callId) callPass.set(callId, view);
        const streamed = callId ? deltas[callId] : undefined;
        if (streamed) {
          view.text = streamed;
          view.streaming = true;
        }
        break;
      }
      case "llm.delta": {
        const callId = callIdOf(event);
        const view = callId ? callPass.get(callId) : undefined;
        if (view) {
          view.text = deltas[callId ?? ""] ?? `${view.text}${str(data.text) ?? ""}`;
          view.streaming = true;
        }
        break;
      }
      case "llm.response": {
        const callId = callIdOf(event);
        const view = callId ? callPass.get(callId) : undefined;
        if (view) {
          const text = str(data.text);
          if (text !== undefined && text.length > 0) view.text = text;
          view.streaming = false;
          const usage = usageOf(event);
          if (usage) {
            view.tokensIn = usage.tokensIn;
            view.tokensOut = usage.tokensOut;
          }
          view.model = str(data.model) ?? event.model ?? view.model;
        }
        break;
      }
      case "llm.error": {
        const callId = callIdOf(event);
        const view = callId ? callPass.get(callId) : undefined;
        if (view) {
          view.status = "failed";
          view.streaming = false;
        }
        break;
      }
      case "pass.completed": {
        const view = upsertPass(turnId, str(data.pass) ?? "pass", num(data.attempt) ?? 1);
        view.status = "completed";
        view.streaming = false;
        view.durationMs = num(data.durationMs);
        const text = str(data.text);
        if (text) view.text = text;
        const usage = usageOf(event);
        if (usage) {
          view.tokensIn = usage.tokensIn;
          view.tokensOut = usage.tokensOut;
        }
        break;
      }
      case "pass.failed": {
        const view = upsertPass(turnId, str(data.pass) ?? "pass", num(data.attempt) ?? 1);
        view.status = "failed";
        view.streaming = false;
        view.durationMs = num(data.durationMs);
        break;
      }
      case "pass.retry": {
        const view = upsertPass(turnId, str(data.pass) ?? "pass", num(data.attempt) ?? 1);
        view.status = "retried";
        view.streaming = false;
        view.retryReason = str(data.reason);
        break;
      }
      case "agent.message": {
        const text = str(data.text) ?? "";
        if (!items.some((item) => item.kind === "assistant" && item.id === turnId)) {
          items.push({ kind: "assistant", id: turnId, text, verified: data.verified === true });
        }
        break;
      }
      case "turn.completed": {
        const existing = items.find((item) => item.kind === "assistant" && item.id === turnId);
        if (existing && existing.kind === "assistant") {
          existing.verified = data.verified === true;
        } else {
          items.push({
            kind: "assistant",
            id: turnId,
            text: str(data.reply) ?? "",
            verified: data.verified === true,
          });
        }
        break;
      }
      case "turn.failed": {
        if (!items.some((item) => item.kind === "error" && item.id === turnId)) {
          items.push({
            kind: "error",
            id: turnId,
            message: str(data.error) ?? "The turn failed.",
            detail: str(data.kind),
          });
        }
        break;
      }
      default:
        break;
    }
  }

  return items;
}

/** Only the items a transcript should render. */
export function visibleItems(items: TranscriptItem[]): TranscriptItem[] {
  return items.filter((item) => item.kind !== "passes" || item.passes.length > 0);
}

/** A stable key for a transcript item. */
export function itemKey(item: TranscriptItem): string {
  return `${item.kind}:${item.id}`;
}
