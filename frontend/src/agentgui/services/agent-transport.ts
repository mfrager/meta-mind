import {
  openChatStream,
  createSession as createChatSession,
} from "@/lib/api/chat.ts";
import type { ChatFrame } from "@/lib/wire/chat.ts";

import type {
  AgentEvent,
  ChatRequestPayload,
  PassName,
  QuestionnairePayload,
  ServerHealth,
} from "@/agentgui/types/workspace";

export const API_BASE = import.meta.env.VITE_API_BASE ?? "";

export type AgentEventHandler = (event: AgentEvent) => void;

export interface AgentTransport {
  readonly mode: "server" | "mock";
  send(
    payload: ChatRequestPayload,
    onEvent: AgentEventHandler,
    signal?: AbortSignal
  ): Promise<void>;
}

/**
 * Local (UI) session ids map onto metamind's server session ids. Sessions the
 * UI loaded from `GET /api/chat/sessions` are already server ids and bind to
 * themselves; sessions created through the sidebar get one on first send.
 */
const bindings = new Map<string, string>();

export function bindSession(localId: string, serverId: string): void {
  bindings.set(localId, serverId);
}

function isUnknownSession(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return /no session|not found|404/i.test(message);
}

function stringify(value: unknown): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function asQuestionnaire(
  id: string,
  request: unknown
): QuestionnairePayload | null {
  if (!request || typeof request !== "object") return null;
  const raw = request as {
    prompt?: unknown;
    question?: unknown;
    options?: unknown;
    multi_select?: unknown;
    multiSelect?: unknown;
  };
  if (!Array.isArray(raw.options) || raw.options.length === 0) return null;
  const options = raw.options.flatMap((option, index) => {
    if (!option || typeof option !== "object") return [];
    const entry = option as {
      id?: unknown;
      value?: unknown;
      label?: unknown;
      description?: unknown;
    };
    const value = entry.value ?? entry.id ?? entry.label;
    if (typeof value !== "string") return [];
    return [
      {
        id: String(entry.id ?? value ?? index),
        label: typeof entry.label === "string" ? entry.label : value,
        description:
          typeof entry.description === "string" ? entry.description : undefined,
      },
    ];
  });
  if (options.length === 0) return null;
  const question =
    typeof raw.prompt === "string"
      ? raw.prompt
      : typeof raw.question === "string"
        ? raw.question
        : "The agent needs your input.";
  return {
    id,
    question,
    options,
    multiSelect: Boolean(raw.multi_select ?? raw.multiSelect),
  };
}

function asConfirmation(id: string, request: unknown) {
  const raw =
    request && typeof request === "object"
      ? (request as { prompt?: unknown; purpose?: unknown; action?: unknown })
      : {};
  return {
    id,
    action:
      typeof raw.action === "string"
        ? raw.action
        : typeof raw.prompt === "string"
          ? raw.prompt
          : "Approval requested",
    detail:
      typeof raw.purpose === "string"
        ? raw.purpose
        : stringify(request) || "The agent is waiting for a decision.",
    accepted: undefined,
  };
}

const PASS_NAMES: readonly PassName[] = ["interpret", "process", "verify", "think"];

function asPass(value: string): PassName | null {
  return (PASS_NAMES as readonly string[]).includes(value)
    ? (value as PassName)
    : null;
}

/** Translate one metamind chat frame into zero or more workspace events. */
export function eventsForFrame(frame: ChatFrame): AgentEvent[] {
  switch (frame.type) {
    case "pass_start": {
      const pass = asPass(frame.pass);
      if (!pass) return [];
      return [
        {
          type: "pass_start",
          pass,
          attempt: frame.attempt,
          model: frame.model ?? undefined,
        },
      ];
    }
    case "pass_round": {
      const pass = asPass(frame.pass);
      if (!pass) return [];
      return [
        {
          type: "pass_round",
          pass,
          attempt: frame.attempt,
          round: frame.round,
          model: frame.model ?? undefined,
          streaming: frame.streaming,
        },
      ];
    }
    case "pass_token": {
      // A final token is the answer, and the terminal frame renders it. Phase
      // prose and (for now) reasoning never become assistant text.
      if (frame.final || frame.channel === "reasoning" || !frame.token) return [];
      const pass = asPass(frame.pass);
      if (!pass) return [];
      return [
        {
          type: "pass_delta",
          pass,
          attempt: frame.attempt ?? 1,
          round: frame.round ?? 1,
          delta: frame.token,
        },
      ];
    }
    case "pass_round_end": {
      const pass = asPass(frame.pass);
      if (!pass) return [];
      return [
        {
          type: "pass_round_end",
          pass,
          attempt: frame.attempt,
          round: frame.round,
          outcome: frame.outcome,
          tokensIn: frame.tokensIn,
          tokensOut: frame.tokensOut,
          latencyMs: frame.latencyMs,
          error: frame.error ?? undefined,
        },
      ];
    }
    case "pass_complete": {
      const pass = asPass(frame.pass);
      if (!pass) return [];
      return [
        {
          type: "pass_complete",
          pass,
          attempt: frame.attempt ?? 1,
          finished: frame.finished,
          budget: frame.budget,
        },
      ];
    }
    case "pass_retry": {
      const pass = asPass(frame.pass);
      if (!pass) return [];
      return [
        {
          type: "pass_retry",
          pass,
          attempt: frame.attempt,
          nextAttempt: frame.nextAttempt,
          reason: frame.reason,
        },
      ];
    }
    case "verification":
      return [
        {
          type: "verification",
          label: frame.label,
          pass: frame.pass,
          confidence: frame.confidence,
          attempt: frame.attempt ?? undefined,
          backend: frame.backend ?? undefined,
          reasoning: frame.reasoning ?? undefined,
          durationMs: frame.durationMs ?? undefined,
        },
      ];
    case "tool_call": {
      const pass = frame.pass ? asPass(frame.pass) : null;
      return [
        {
          type: "tool",
          call: {
            id: frame.id,
            name: frame.name,
            status: "running",
            description: pass
              ? `${pass} · round ${frame.round ?? 1}`
              : frame.phase,
            input: frame.input,
            pass: pass ?? undefined,
            attempt: frame.attempt ?? undefined,
            round: frame.round ?? undefined,
            action: frame.action ?? undefined,
          },
        },
      ];
    }
    case "tool_result": {
      const pass = frame.pass ? asPass(frame.pass) : null;
      return [
        {
          type: "tool",
          call: {
            id: frame.id,
            name: frame.name,
            status: frame.success ? "done" : "error",
            output: stringify(frame.output),
            input: undefined,
            pass: pass ?? undefined,
            attempt: frame.attempt ?? undefined,
            round: frame.round ?? undefined,
          },
        },
      ];
    }
    case "llm_response": {
      if (!frame.visible || frame.is_final || !frame.text.trim()) return [];
      return [{ type: "text", delta: `${frame.text.trim()}\n\n` }];
    }
    case "input_request": {
      const questionnaire = asQuestionnaire(frame.id, frame.request);
      if (questionnaire) {
        return [{ type: "questionnaire", payload: questionnaire }];
      }
      return [
        { type: "confirmation", payload: asConfirmation(frame.id, frame.request) },
      ];
    }
    case "final_response":
      return frame.text.trim()
        ? [{ type: "text", delta: frame.text.trim() }]
        : [];
    case "failed":
      return [{ type: "error", message: frame.reason ?? frame.error }];
    case "completed":
      return [{ type: "done", finishReason: "stop" }];
    default:
      return [];
  }
}

class ServerTransport implements AgentTransport {
  readonly mode = "server" as const;

  async send(
    payload: ChatRequestPayload,
    onEvent: AgentEventHandler,
    signal?: AbortSignal
  ): Promise<void> {
    const localId = payload.sessionId;
    const bound = bindings.get(localId);
    const emit = (entry: { frame: ChatFrame }) => {
      for (const event of eventsForFrame(entry.frame)) onEvent(event);
    };

    onEvent({
      type: "start",
      messageId: `mm_${Date.now().toString(36)}`,
      model: payload.model,
    });

    try {
      await openChatStream(
        { sessionId: bound ?? localId, query: payload.prompt },
        emit,
        signal
      );
    } catch (error) {
      if (!isUnknownSession(error) || bound) throw error;
      // The sidebar created this session locally; metamind needs one first.
      const created = await createChatSession();
      bindings.set(localId, created.id);
      await openChatStream(
        { sessionId: created.id, query: payload.prompt },
        emit,
        signal
      );
    }
  }
}

const wait = (ms: number, signal?: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    if (signal?.aborted) {
      reject(new DOMException("Aborted", "AbortError"));
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    function onAbort() {
      clearTimeout(timer);
      reject(new DOMException("Aborted", "AbortError"));
    }
    signal?.addEventListener("abort", onAbort, { once: true });
  });

function chunkWords(text: string, size: number): string[] {
  const words = text.match(/\S+\s*/g) ?? [text];
  const chunks: string[] = [];
  for (let i = 0; i < words.length; i += size) {
    chunks.push(words.slice(i, i + size).join(""));
  }
  return chunks;
}

class MockTransport implements AgentTransport {
  readonly mode = "mock" as const;

  async send(
    payload: ChatRequestPayload,
    onEvent: AgentEventHandler,
    signal?: AbortSignal
  ): Promise<void> {
    onEvent({ type: "start", messageId: `mock_${Date.now().toString(36)}`, model: payload.model });

    const body = `This is the local mock stream, used when the metamind backend is unreachable.\n\nYou asked: **${payload.prompt.trim()}**\n\nStart \`mm-serve\` and switch the transport back to the backend to get real answers.`;
    for (const token of chunkWords(body, 4)) {
      await wait(18, signal);
      onEvent({ type: "text", delta: token });
    }

    await wait(80, signal);
    onEvent({ type: "done", finishReason: "stop" });
  }
}

export function createTransport(mode: AgentTransport["mode"]): AgentTransport {
  return mode === "mock" ? new MockTransport() : new ServerTransport();
}

export async function checkHealth(): Promise<ServerHealth | null> {
  try {
    const response = await fetch(`${API_BASE}/api/health`, {
      signal: AbortSignal.timeout(2_500),
    });
    if (!response.ok) return null;
    const body = (await response.json()) as {
      ok?: boolean;
      wireVersion?: number;
      kernel?: { state?: string };
    };
    return {
      status: body.ok ? "ok" : "degraded",
      backend: "mm-serve",
      model: `kernel ${body.kernel?.state ?? "unknown"}`,
      demo: false,
      version: body.wireVersion ? `wire ${body.wireVersion}` : "unknown",
    };
  } catch {
    return null;
  }
}

export async function fetchModels(): Promise<string[]> {
  return [];
}
