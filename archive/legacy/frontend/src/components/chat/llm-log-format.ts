import type { CallGroup, LlmLogRow, RequestRow } from "@/lib/store/llm-log.ts";
import type { LlmActivityEvent } from "@/lib/wire/llm_activity.ts";

type ResponseEvent = Extract<LlmActivityEvent, { kind: "response" }>;
type ErrorEvent = Extract<LlmActivityEvent, { kind: "error" }>;
export type OutcomeRow = ResponseEvent | ErrorEvent;
type RowKind = LlmLogRow["kind"];

export const KIND_LABEL: Record<RowKind, string> = {
  request: "REQ",
  delta: "STR",
  response: "RSP",
  error: "ERR",
};

export const KIND_CLASS: Record<RowKind, string> = {
  request: "text-azure-600",
  delta: "text-muted-foreground",
  response: "text-emerald-600",
  error: "text-red-600",
};

export function clock(at: number) {
  return new Date(at).toLocaleTimeString("en-GB", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function shortCallId(callId: string) {
  return callId.slice(0, 8);
}

/** `chat/process r2` — the origin tuple Phase 1 started populating. */
export function sourceTuple(row: RequestRow) {
  const pass = row.pass ? `/${row.pass}` : "";
  const round = row.round === null || row.round === undefined ? "" : ` r${row.round}`;
  return `${row.source}${pass}${round}`;
}

export function requestSummary(row: RequestRow) {
  return row.streamText || row.preview || "(empty prompt)";
}

export function requestMeta(row: RequestRow) {
  const parts: string[] = [sourceTuple(row), row.purpose];
  if (row.trigger) parts.push(`trigger=${row.trigger}`);
  if (row.tool) parts.push(`tool=${row.tool}`);
  if (row.parentCallId) parts.push(`parent=${shortCallId(row.parentCallId)}`);
  if (row.step) parts.push(`step=${row.step}`);
  if (row.origin) parts.push(`origin=${row.origin}`);
  if (row.attempt > 1) parts.push(`attempt=${row.attempt}`);
  if (row.model) parts.push(row.model);
  parts.push(`${row.messageCount} msgs`, `${row.toolCount} tools`);
  if (row.streamed) parts.push("stream");
  if (row.streamChars > 0) parts.push(`${row.streamChars} chars`);
  return parts.join(" · ");
}

/** The body character count that explains the cost, before the server-side cap. */
export function outcomeChars(outcome: OutcomeRow) {
  if (outcome.kind === "response") return outcome.streamChars || outcome.streamText.length;
  return outcome.streamChars || outcome.partialChars;
}

export function outcomeSummary(outcome: OutcomeRow, request: RequestRow) {
  if (outcome.kind === "response") {
    return (
      outcome.streamText ||
      outcome.toolPreview ||
      request.streamText ||
      outcome.text ||
      "(empty reply)"
    );
  }
  return outcome.error;
}

/** `reasoning_only · length · 3838 reasoning chars` — the reported failure, readable. */
export function outcomeMeta(outcome: OutcomeRow) {
  if (outcome.kind === "response") {
    const parts: string[] = [outcome.model, outcome.status];
    if (outcome.cached) parts.push("cached");
    parts.push(`${outcome.tokensIn}/${outcome.tokensOut} tok`, `${outcome.latencyMs} ms`);
    // A tool-call answer has no content characters; `0 chars` next to "1 tool
    // calls" reads as an empty reply even when the reply is in the call.
    if (outcomeChars(outcome) > 0 || outcome.toolCalls === 0) {
      parts.push(`${outcomeChars(outcome)} chars`);
    }
    if (outcome.streamTruncated) parts.push("truncated");
    if (outcome.reasoningChars > 0) parts.push(`${outcome.reasoningChars} reasoning chars`);
    if (outcome.toolCalls > 0) parts.push(`${outcome.toolCalls} tool calls`);
    return parts.join(" · ");
  }

  const parts: string[] = [outcome.status];
  if (outcome.finishReason) parts.push(outcome.finishReason);
  if (outcome.reasoningChars > 0) parts.push(`${outcome.reasoningChars} reasoning chars`);
  parts.push(`${outcome.latencyMs} ms`);
  const chars = outcomeChars(outcome);
  if (chars > 0) parts.push(`${chars} chars`);
  return parts.join(" · ");
}

/** Collapse control characters and runs of whitespace so one block is one line. */
export function normalizeLine(text: string) {
  let out = "";
  for (const char of text) {
    const code = char.charCodeAt(0);
    out += code < 0x20 || code === 0x7f ? " " : char;
  }
  return out.replace(/\s+/g, " ").trim();
}

/** Every tool call the model proposed, or empty when it proposed none. */
export function outcomeTools(outcome: OutcomeRow) {
  if (outcome.kind !== "response" || outcome.toolCalls === 0) return "";
  return outcome.toolPreview || `${outcome.toolCalls} tool calls`;
}

export function outcomeBody(outcome: OutcomeRow, request: RequestRow) {
  if (outcome.kind === "response") {
    const text = outcome.streamText || request.streamText;
    const tools = outcomeTools(outcome);
    if (text && tools) return `${text} · tools: ${tools}`;
    return text || tools;
  }
  return "";
}

export type LlmTextBlock = {
  key: string;
  kind: "request" | "response" | "error";
  depth: number;
  /** Exactly three lines: identity, numbers, one-line text. */
  lines: [string, string, string];
};

function requestIdentity(request: RequestRow) {
  const parts: string[] = [sourceTuple(request), request.purpose];
  if (request.trigger) {
    const trigger = [`trigger=${request.trigger}`];
    if (request.tool) trigger.push(`tool=${request.tool}`);
    if (request.parentCallId) trigger.push(`parent=${shortCallId(request.parentCallId)}`);
    parts.push(trigger.join(" "));
  } else if (request.tool) {
    parts.push(`tool=${request.tool}`);
  }
  if (request.origin) parts.push(`origin=${request.origin}`);
  if (request.step) parts.push(`step=${request.step}`);
  if (request.attempt > 1) parts.push(`attempt=${request.attempt}`);
  return parts.join(" · ");
}

function requestNumbers(request: RequestRow) {
  const parts: string[] = [];
  if (request.model) parts.push(request.model);
  parts.push("requested", `${request.messageCount} msgs`, `${request.toolCount} tools`);
  if (request.streamed) parts.push("stream");
  if (request.streamChars > 0) parts.push(`${request.streamChars} chars`);
  return parts.join(" · ");
}

function quote(text: string) {
  return `"${normalizeLine(text)}"`;
}

function requestBlock(request: RequestRow): LlmTextBlock {
  return {
    key: `${request.callId}:request`,
    kind: "request",
    depth: 0,
    lines: [
      `${clock(request.at)} REQ ${shortCallId(request.callId)} · ${requestIdentity(request)}`,
      requestNumbers(request),
      quote(requestSummary(request)),
    ],
  };
}

function outcomeBlock(outcome: OutcomeRow, request: RequestRow): LlmTextBlock {
  const text =
    outcome.kind === "response"
      ? outcomeBody(outcome, request) || outcomeSummary(outcome, request)
      : outcome.error;
  return {
    key: `${outcome.callId}:outcome`,
    kind: outcome.kind,
    depth: 0,
    lines: [
      `${clock(outcome.at)} ${outcome.kind === "response" ? "RSP" : "ERR"} ${shortCallId(outcome.callId)} · ${requestIdentity(request)}`,
      outcomeMeta(outcome),
      quote(text),
    ],
  };
}

function indent(block: LlmTextBlock, depth: number): LlmTextBlock {
  const pad = "  ".repeat(depth);
  const [first, second, third] = block.lines;
  const marker = block.kind === "request" ? "↳REQ" : block.kind === "response" ? "↳RSP" : "↳ERR";
  return {
    ...block,
    key: `${block.key}:${depth}`,
    depth,
    lines: [pad + first.replace(/ (REQ|RSP|ERR) /, ` ${marker} `), pad + second, pad + third],
  };
}

/** One block per request and one per outcome; deltas never produce a block. */
export function textBlocks(groups: CallGroup[]): LlmTextBlock[] {
  const blocks: LlmTextBlock[] = [];
  const push = (group: CallGroup, depth: number) => {
    blocks.push(depth === 0 ? requestBlock(group.request) : indent(requestBlock(group.request), depth));
    if (group.outcome) blocks.push(indent(outcomeBlock(group.outcome, group.request), depth + 1));
    for (const child of group.children) push(child, depth + 1);
  };
  for (const group of groups) push(group, 0);
  return blocks;
}

/** The copy payload: the same three-line blocks, with full (unclipped) text. */
export function textLog(groups: CallGroup[]): string {
  return textBlocks(groups)
    .flatMap((block) => block.lines)
    .join("\n");
}

export function detailRows(row: LlmLogRow, group?: CallGroup): Array<[string, string]> {
  const rows: Array<[string, string]> = [
    ["seq", String(row.seq)],
    ["call", row.callId],
  ];
  const add = (label: string, value: string | number | boolean | null | undefined) => {
    if (value === null || value === undefined || value === "") return;
    rows.push([label, typeof value === "boolean" ? (value ? "yes" : "no") : String(value)]);
  };

  switch (row.kind) {
    case "request":
      add("trace", row.traceId);
      add("session", row.sessionId);
      add("run", row.runId);
      add("pass", row.pass);
      add("round", row.round);
      add("source", row.source);
      add("purpose", row.purpose);
      add("trigger", row.trigger);
      add("parent", row.parentCallId);
      add("step", row.step);
      add("tool", row.tool);
      add("origin", row.origin);
      add("attempt", row.attempt);
      add("timeout", row.timeoutMs ? `${row.timeoutMs} ms` : null);
      add("orphan", group?.orphan);
      add("provider", row.provider);
      add("model", row.model);
      add("requested", row.requested);
      add("streamed", row.streamed);
      add("messages", row.messageCount);
      add("tools", row.toolCount);
      add("prompt", row.preview);
      break;
    case "delta":
      add("chars", row.chars);
      add("truncated", row.truncated);
      add("text", row.text);
      break;
    case "response":
      add("model", row.model);
      add("status", row.status);
      add("cached", row.cached);
      add("tokens in", row.tokensIn);
      add("tokens out", row.tokensOut);
      add("latency", `${row.latencyMs} ms`);
      add("tool calls", row.toolCalls);
      add("tool preview", row.toolPreview);
      add("attempt", row.attempt);
      add("stream chars", row.streamChars);
      add("stream truncated", row.streamTruncated);
      add("reasoning chars", row.reasoningChars);
      add("reply preview", row.text);
      break;
    case "error":
      add("status", row.status);
      add("latency", `${row.latencyMs} ms`);
      add("attempt", row.attempt);
      add("partial chars", row.partialChars);
      add("stream chars", row.streamChars);
      add("reasoning chars", row.reasoningChars);
      add("finish reason", row.finishReason);
      add("error", row.error);
      break;
  }
  return rows;
}
