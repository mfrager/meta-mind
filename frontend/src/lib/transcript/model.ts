import type { Event, EventType } from "@/lib/wire/agentstream.ts";

/** The persisted event names, duplicated from the generated union for runtime use. */
export const EVENT_TYPES = [
  "session.started",
  "session.ended",
  "turn.started",
  "turn.completed",
  "turn.failed",
  "pass.started",
  "pass.completed",
  "pass.failed",
  "pass.retry",
  "prompt.rendered",
  "llm.request",
  "llm.attempt",
  "llm.delta",
  "llm.response",
  "llm.error",
  "tool.call",
  "tool.result",
  "state.updated",
  "verification.registered",
  "verification.result",
  "internal.action",
  "internal.error",
  "agent.message",
  "user.message",
] as const satisfies readonly EventType[];

export type JsonObject = Record<string, unknown>;

/** The payload as an object. Every event this core writes has an object payload. */
export function dataOf(event: Event): JsonObject {
  return event.data && typeof event.data === "object" && !Array.isArray(event.data)
    ? (event.data as JsonObject)
    : {};
}

export function str(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

export function num(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

export function bool(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

export function list(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

export function object(value: unknown): JsonObject {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonObject) : {};
}

export function passOf(event: Event): string | undefined {
  return str(dataOf(event).pass);
}

export function callIdOf(event: Event): string | undefined {
  return str(dataOf(event).callId);
}

export function usageOf(event: Event): { tokensIn: number; tokensOut: number } | undefined {
  const usage = object(dataOf(event).usage);
  const tokensIn = num(usage.tokensIn);
  const tokensOut = num(usage.tokensOut);
  if (tokensIn === undefined && tokensOut === undefined) return undefined;
  return { tokensIn: tokensIn ?? 0, tokensOut: tokensOut ?? 0 };
}

const ERROR_TYPES: ReadonlySet<EventType> = new Set(["turn.failed", "pass.failed", "llm.error", "internal.error"]);

/** Whether the record reports a failure. */
export function isErrorEvent(event: Event): boolean {
  if (ERROR_TYPES.has(event.type)) return true;
  if (event.type === "tool.result") return dataOf(event).ok === false;
  return false;
}

/** A compact one-line summary for the dock row. */
export function recordSummary(event: Event): string {
  const data = dataOf(event);
  switch (event.type) {
    case "session.started":
      return str(data.title) ?? `session ${event.sessionId.slice(0, 8)}`;
    case "session.ended":
      return str(data.reason) ?? "ended";
    case "turn.started":
    case "user.message":
      return str(data.text) ?? str(data.userInput) ?? "";
    case "turn.completed":
      return str(data.reply) ?? "";
    case "turn.failed":
      return str(data.error) ?? "turn failed";
    case "pass.started":
      return `${str(data.pass) ?? "pass"} attempt ${num(data.attempt) ?? 1}${event.model ? ` · ${event.model}` : ""}`;
    case "pass.completed":
      return `${str(data.pass) ?? "pass"} · ${str(data.finished) ?? "done"} · ${num(data.durationMs) ?? 0} ms`;
    case "pass.failed":
      return `${str(data.pass) ?? "pass"} · ${str(data.error) ?? "failed"}`;
    case "pass.retry":
      return `${str(data.pass) ?? "pass"} · retry ${num(data.attempt) ?? 0}→${num(data.nextAttempt) ?? 0}`;
    case "prompt.rendered":
      return `${str(data.strategy) ?? "prompt"} · ${str(data.hash) ?? ""} · ${num(data.messageCount) ?? 0} msgs`;
    case "llm.request":
      return `${str(data.purpose) ?? "call"} · ${num(data.messageCount) ?? 0} msgs · ${num(data.toolCount) ?? 0} tools${event.model ? ` · ${event.model}` : ""}`;
    case "llm.attempt":
      return `attempt ${num(data.attemptNo) ?? 1} · ${str(data.status) ?? ""} · ${num(data.latencyMs) ?? 0} ms`;
    case "llm.delta":
      return str(data.text) ?? "";
    case "llm.response":
      return `${str(data.model) ?? ""} · ${num(data.latencyMs) ?? 0} ms · ${str(data.text)?.slice(0, 80) ?? "(tool call)"}`;
    case "llm.error":
      return `${str(data.kind) ?? "error"} · ${str(data.message) ?? ""}`;
    case "tool.call":
      return `${str(data.name) ?? "tool"} ${str(data.arguments) ?? ""}`;
    case "tool.result":
      return `${str(data.name) ?? "tool"} · ${data.ok === false ? "failed" : "ok"} · ${num(data.durationMs) ?? 0} ms`;
    case "state.updated":
      return `revision ${num(data.revision) ?? 0}`;
    case "verification.registered":
      return `${list(data.checks).length} checks`;
    case "verification.result":
      return `${str(data.label) ?? "check"} · ${data.pass === true ? "pass" : "fail"} · ${str(data.backend) ?? ""}`;
    case "internal.action":
      return `${str(data.component) ?? ""} · ${str(data.action) ?? ""} · ${data.ok === false ? "failed" : "ok"}`;
    case "internal.error":
      return `${str(data.component) ?? ""} · ${str(data.action) ?? ""} · ${str(data.message) ?? ""}`;
    case "agent.message":
      return str(data.text) ?? "";
    default:
      return "";
  }
}

/** One line per record for copy-as-text. */
export function copyText(records: Event[]): string {
  return records
    .map((event) => `${event.seq}\t${event.type}\t${passOf(event) ?? "-"}\t${recordSummary(event)}`)
    .join("\n");
}

/** The exact request payload, pretty-printed. The viewer never re-truncates it. */
export function requestJson(event: Event): string {
  return JSON.stringify(event.data, null, 2);
}

export function clock(at: number): string {
  return new Date(at).toLocaleTimeString("en-GB", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
