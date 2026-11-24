import { create } from "zustand";

import {
  maybeRetitleSession,
  useWorkspaceStore,
} from "@/agentgui/lib/store/workspace-store";
import { createId } from "@/agentgui/lib/utils";
import { createTransport } from "@/agentgui/services/agent-transport";
import type {
  AgentEvent,
  ChatMessage,
  MessagePart,
  PassName,
  PassPart,
  TraceStep,
} from "@/agentgui/types/workspace";

const controllers = new Map<string, AbortController>();
const activeAssistant = new Map<string, string>();

interface ChatState {
  messagesBySession: Record<string, ChatMessage[]>;
  streamingSessions: Record<string, boolean>;
  errors: Record<string, string | null>;
  queue: Record<string, string[]>;

  send: (sessionId: string, text: string) => void;
  stop: (sessionId: string) => void;
  retry: (sessionId: string) => void;
  editAndResend: (sessionId: string, messageId: string, text: string) => void;
  removeQueued: (sessionId: string, index: number) => void;
  clearSession: (sessionId: string) => void;
}

function patchMessage(
  sessionId: string,
  messageId: string,
  updater: (message: ChatMessage) => ChatMessage
) {
  useChatStore.setState((state) => {
    const messages = state.messagesBySession[sessionId] ?? [];
    return {
      messagesBySession: {
        ...state.messagesBySession,
        [sessionId]: messages.map((message) =>
          message.id === messageId ? updater(message) : message
        ),
      },
    };
  });
}

function markThinkingDone(parts: MessagePart[]): MessagePart[] {
  return parts.map((part, index) =>
    part.type === "thinking" && index === parts.length - 1
      ? { ...part, done: true }
      : part
  );
}

/**
 * Index of the last part of `type` in the trailing run of thinking/text parts.
 * Providers can interleave reasoning and answer deltas, so a delta of the same
 * kind may sit just before the end of the message. Tool calls, plans, sources
 * and other part types still act as boundaries and start a new block.
 */
function trailingPartIndex(
  parts: MessagePart[],
  type: "thinking" | "text"
): number {
  for (let index = parts.length - 1; index >= 0; index -= 1) {
    const part = parts[index];
    if (part.type === type) return index;
    if (part.type !== "thinking" && part.type !== "text") return -1;
  }
  return -1;
}

function emptyPass(pass: PassName, attempt: number): PassPart {
  return {
    type: "pass",
    pass,
    attempt,
    status: "running",
    rounds: [],
    tools: [],
    checks: [],
  };
}

function passIndex(parts: MessagePart[], pass: PassName, attempt: number): number {
  return parts.findLastIndex(
    (part) => part.type === "pass" && part.pass === pass && part.attempt === attempt
  );
}

/** Find a pass card, creating it when frames arrive out of order or late. */
function ensurePass(
  parts: MessagePart[],
  pass: PassName,
  attempt: number
): { parts: MessagePart[]; index: number } {
  const index = passIndex(parts, pass, attempt);
  if (index !== -1) return { parts, index };
  return { parts: [...parts, emptyPass(pass, attempt)], index: parts.length };
}

function updatePass(
  parts: MessagePart[],
  pass: PassName,
  attempt: number,
  updater: (part: PassPart) => PassPart
): MessagePart[] {
  const ensured = ensurePass(parts, pass, attempt);
  return ensured.parts.map((part, index) =>
    index === ensured.index && part.type === "pass" ? updater(part) : part
  );
}

function updateRound(
  part: PassPart,
  round: number,
  updater: (round: PassPart["rounds"][number]) => PassPart["rounds"][number]
): PassPart {
  const index = part.rounds.findIndex((entry) => entry.round === round);
  if (index === -1) {
    return {
      ...part,
      rounds: [
        ...part.rounds,
        updater({ round, streaming: false, text: "" }),
      ],
    };
  }
  return {
    ...part,
    rounds: part.rounds.map((entry, entryIndex) =>
      entryIndex === index ? updater(entry) : entry
    ),
  };
}

function stepFor(
  call: Extract<AgentEvent, { type: "tool" }>["call"],
  existing?: TraceStep,
  now = Date.now()
): TraceStep {
  return {
    id: call.id,
    kind: "tool",
    title: call.name,
    description: call.description,
    status: call.status,
    startedAt: existing?.startedAt ?? now,
    input: call.input ?? existing?.input,
    output: call.output ?? existing?.output,
    durationMs:
      call.status === "done"
        ? existing?.startedAt
          ? Math.max(120, now - existing.startedAt)
          : (existing?.durationMs ?? 320)
        : existing?.durationMs,
  };
}

function upsertPassTool(
  part: PassPart,
  call: Extract<AgentEvent, { type: "tool" }>["call"]
): PassPart {
  const index = part.tools.findIndex((entry) => entry.id === call.id);
  if (index === -1) return { ...part, tools: [...part.tools, stepFor(call)] };
  return {
    ...part,
    tools: part.tools.map((entry, entryIndex) =>
      entryIndex === index ? stepFor(call, entry) : entry
    ),
  };
}

function settlePasses(parts: MessagePart[], failed = false): MessagePart[] {
  return parts.map((part) =>
    part.type === "pass" && part.status === "running"
      ? { ...part, status: failed ? "failed" : "done" }
      : part
  );
}

function failedFinish(finished: string): boolean {
  return /error|failed|rejected|exhausted/.test(finished);
}

function upsertToolStep(parts: MessagePart[], event: Extract<AgentEvent, { type: "tool" }>): MessagePart[] {
  const { call } = event;
  const traceIndex = parts.findLastIndex((part) => part.type === "trace");
  const now = Date.now();

  const step: TraceStep = {
    id: call.id,
    kind: "tool",
    title: call.name,
    description: call.description,
    status: call.status,
    startedAt: now,
    input: call.input,
    output: call.output,
    durationMs: call.status === "done" ? 320 : undefined,
  };

  if (traceIndex === -1) {
    return [...parts, { type: "trace", steps: [step] }];
  }

  return parts.map((part, index) => {
    if (index !== traceIndex || part.type !== "trace") return part;
    const existingIndex = part.steps.findIndex((entry) => entry.id === call.id);
    if (existingIndex === -1) {
      return { ...part, steps: [...part.steps, step] };
    }
    const steps = part.steps.map((entry, entryIndex) =>
      entryIndex === existingIndex
        ? {
            ...entry,
            status: call.status,
            output: call.output ?? entry.output,
            description: call.description ?? entry.description,
            input: call.input ?? entry.input,
            durationMs:
              call.status === "done"
                ? entry.startedAt
                  ? Math.max(120, now - entry.startedAt)
                  : entry.durationMs
                : entry.durationMs,
          }
        : entry
    );
    return { ...part, steps };
  });
}

export function applyEvent(message: ChatMessage, event: AgentEvent): ChatMessage {
  switch (event.type) {
    case "start":
      return { ...message, model: event.model };
    case "thinking": {
      const parts = [...message.parts];
      const index = trailingPartIndex(parts, "thinking");
      if (index !== -1 && parts[index].type === "thinking") {
        const part = parts[index];
        parts[index] = { ...part, text: part.text + event.delta, done: false };
      } else {
        parts.push({ type: "thinking", text: event.delta });
      }
      return { ...message, parts };
    }
    case "text": {
      const parts = markThinkingDone([...message.parts]);
      const index = trailingPartIndex(parts, "text");
      if (index !== -1 && parts[index].type === "text") {
        const part = parts[index];
        parts[index] = { ...part, text: part.text + event.delta };
      } else {
        parts.push({ type: "text", text: event.delta });
      }
      return { ...message, parts };
    }
    case "tool": {
      if (event.call.pass) {
        return {
          ...message,
          parts: updatePass(
            message.parts,
            event.call.pass,
            event.call.attempt ?? 1,
            (part) => upsertPassTool(part, event.call)
          ),
        };
      }
      return { ...message, parts: upsertToolStep(message.parts, event) };
    }
    case "pass_start": {
      const ensured = ensurePass(message.parts, event.pass, event.attempt);
      const parts = ensured.parts.map((part, index) =>
        index === ensured.index && part.type === "pass"
          ? { ...part, status: "running" as const, model: event.model }
          : part
      );
      return { ...message, parts };
    }
    case "pass_round":
      return {
        ...message,
        parts: updatePass(message.parts, event.pass, event.attempt, (part) =>
          updateRound(part, event.round, (round) => ({
            ...round,
            model: event.model,
            streaming: event.streaming,
          }))
        ),
      };
    case "pass_delta":
      return {
        ...message,
        parts: updatePass(message.parts, event.pass, event.attempt, (part) =>
          updateRound(part, event.round, (round) => ({
            ...round,
            text: round.text + event.delta,
            streaming: true,
          }))
        ),
      };
    case "pass_round_end":
      return {
        ...message,
        parts: updatePass(message.parts, event.pass, event.attempt, (part) =>
          updateRound(part, event.round, (round) => ({
            ...round,
            streaming: false,
            outcome: event.outcome,
            error: event.error,
            tokensIn: event.tokensIn,
            tokensOut: event.tokensOut,
            latencyMs: event.latencyMs,
          }))
        ),
      };
    case "pass_complete":
      return {
        ...message,
        parts: updatePass(message.parts, event.pass, event.attempt, (part) => ({
          ...part,
          status: failedFinish(event.finished) ? "failed" : "done",
          finished: event.finished,
          budget: event.budget,
        })),
      };
    case "pass_retry":
      return {
        ...message,
        parts: updatePass(message.parts, event.pass, event.attempt, (part) => ({
          ...part,
          retry: { nextAttempt: event.nextAttempt, reason: event.reason },
        })),
      };
    case "verification": {
      const attempt = event.attempt;
      const index = message.parts.findLastIndex(
        (part) =>
          part.type === "pass" &&
          part.pass === "verify" &&
          (attempt === undefined || part.attempt === attempt)
      );
      if (index === -1) return message;
      return {
        ...message,
        parts: message.parts.map((part, entryIndex) =>
          entryIndex === index && part.type === "pass"
            ? {
                ...part,
                checks: [
                  ...part.checks,
                  {
                    label: event.label,
                    pass: event.pass,
                    confidence: event.confidence,
                    backend: event.backend,
                    reasoning: event.reasoning,
                    durationMs: event.durationMs,
                  },
                ],
              }
            : part
        ),
      };
    }
    case "plan": {
      const parts = message.parts.filter(
        (part) => !(part.type === "plan" && part.plan.title === event.plan.title)
      );
      return { ...message, parts: [...parts, { type: "plan", plan: event.plan }] };
    }
    case "sources":
      return {
        ...message,
        parts: [
          ...message.parts.filter((part) => part.type !== "sources"),
          { type: "sources", sources: event.sources },
        ],
      };
    case "questionnaire":
      return {
        ...message,
        parts: [...message.parts, { type: "questionnaire", payload: event.payload }],
      };
    case "confirmation":
      return {
        ...message,
        parts: [...message.parts, { type: "confirmation", payload: event.payload }],
      };
    case "subagent": {
      const existing = message.parts.findIndex(
        (part) => part.type === "subagent" && part.name === event.name
      );
      const part: MessagePart = {
        type: "subagent",
        name: event.name,
        model: event.model,
        status: event.status,
        steps: event.steps ?? [],
      };
      if (existing === -1) {
        return { ...message, parts: [...message.parts, part] };
      }
      return {
        ...message,
        parts: message.parts.map((entry, index) =>
          index === existing ? part : entry
        ),
      };
    }
    case "usage":
      return { ...message, usage: event.usage };
    case "done":
      return {
        ...message,
        status: "complete",
        parts: settlePasses(markThinkingDone(message.parts)),
        usage: event.usage ?? message.usage,
      };
    case "error":
      return {
        ...message,
        status: "error",
        error: event.message,
        parts: settlePasses(markThinkingDone(message.parts), true),
      };
    case "artifact":
      return message;
    default:
      return message;
  }
}

export const useChatStore = create<ChatState>()((set, get) => ({
  messagesBySession: {},
  streamingSessions: {},
  errors: {},
  queue: {},

  send: (sessionId, text) => {
    const prompt = text.trim();
    if (!prompt) return;

    if (get().streamingSessions[sessionId]) {
      set((state) => ({
        queue: {
          ...state.queue,
          [sessionId]: [...(state.queue[sessionId] ?? []), prompt],
        },
      }));
      return;
    }

    const userMessage: ChatMessage = {
      id: createId("m"),
      role: "user",
      createdAt: new Date().toISOString(),
      status: "complete",
      parts: [{ type: "text", text: prompt }],
    };

    const assistantMessage: ChatMessage = {
      id: createId("m"),
      role: "assistant",
      createdAt: new Date().toISOString(),
      status: "streaming",
      parts: [],
    };

    set((state) => ({
      messagesBySession: {
        ...state.messagesBySession,
        [sessionId]: [
          ...(state.messagesBySession[sessionId] ?? []),
          userMessage,
          assistantMessage,
        ],
      },
      streamingSessions: { ...state.streamingSessions, [sessionId]: true },
      errors: { ...state.errors, [sessionId]: null },
    }));

    maybeRetitleSession(sessionId, prompt);
    activeAssistant.set(sessionId, assistantMessage.id);
    void runTurn(sessionId, assistantMessage.id, prompt);
  },

  stop: (sessionId) => {
    controllers.get(sessionId)?.abort();
    controllers.delete(sessionId);
    const assistantId = activeAssistant.get(sessionId);
    if (assistantId) {
      patchMessage(sessionId, assistantId, (message) => ({
        ...message,
        status: "cancelled",
        parts: markThinkingDone(message.parts),
      }));
    }
    set((state) => ({
      streamingSessions: { ...state.streamingSessions, [sessionId]: false },
      queue: { ...state.queue, [sessionId]: [] },
    }));
  },

  retry: (sessionId) => {
    if (get().streamingSessions[sessionId]) return;
    const messages = get().messagesBySession[sessionId] ?? [];
    const lastUser = [...messages].reverse().find((message) => message.role === "user");
    if (!lastUser) return;
    const text = lastUser.parts.find((part) => part.type === "text")?.text ?? "";
    get().editAndResend(sessionId, lastUser.id, text);
  },

  editAndResend: (sessionId, messageId, text) => {
    const prompt = text.trim();
    if (!prompt) return;
    const messages = get().messagesBySession[sessionId] ?? [];
    const index = messages.findIndex((message) => message.id === messageId);
    if (index === -1) return;

    if (get().streamingSessions[sessionId]) {
      controllers.get(sessionId)?.abort();
    }

    const retitled: ChatMessage = {
      ...messages[index],
      parts: [{ type: "text", text: prompt }],
    };
    const assistantMessage: ChatMessage = {
      id: createId("m"),
      role: "assistant",
      createdAt: new Date().toISOString(),
      status: "streaming",
      parts: [],
    };

    set((state) => ({
      messagesBySession: {
        ...state.messagesBySession,
        [sessionId]: [...messages.slice(0, index), retitled, assistantMessage],
      },
      streamingSessions: { ...state.streamingSessions, [sessionId]: true },
      errors: { ...state.errors, [sessionId]: null },
    }));

    activeAssistant.set(sessionId, assistantMessage.id);
    void runTurn(sessionId, assistantMessage.id, prompt);
  },

  removeQueued: (sessionId, index) =>
    set((state) => ({
      queue: {
        ...state.queue,
        [sessionId]: (state.queue[sessionId] ?? []).filter(
          (_, entryIndex) => entryIndex !== index
        ),
      },
    })),

  clearSession: (sessionId) =>
    set((state) => ({
      messagesBySession: { ...state.messagesBySession, [sessionId]: [] },
      streamingSessions: { ...state.streamingSessions, [sessionId]: false },
      errors: { ...state.errors, [sessionId]: null },
      queue: { ...state.queue, [sessionId]: [] },
    })),
}));

async function runTurn(
  sessionId: string,
  assistantId: string,
  prompt: string
): Promise<void> {
  const settings = useWorkspaceStore.getState();
  const transport = createTransport(settings.transport);
  const controller = new AbortController();
  controllers.set(sessionId, controller);

  try {
    await transport.send(
      {
        sessionId,
        prompt,
        model: settings.model,
        system: settings.systemPrompt,
        reasoningEffort: settings.reasoningEffort,
      },
      (event) => {
        if (event.type === "artifact") {
          useWorkspaceStore.getState().openArtifact(event.artifact, {
            activate: true,
          });
          return;
        }
        patchMessage(sessionId, assistantId, (message) =>
          applyEvent(message, event)
        );
      },
      controller.signal
    );
  } catch (error) {
    const aborted =
      error instanceof DOMException && error.name === "AbortError";
    patchMessage(sessionId, assistantId, (message) => ({
      ...message,
      status: aborted ? "cancelled" : "error",
      error: aborted
        ? undefined
        : error instanceof Error
          ? error.message
          : "The agent request failed",
      parts: markThinkingDone(message.parts),
    }));
    if (!aborted) {
      useChatStore.setState((state) => ({
        errors: {
          ...state.errors,
          [sessionId]: error instanceof Error ? error.message : "Request failed",
        },
      }));
    }
  } finally {
    controllers.delete(sessionId);
    useChatStore.setState((state) => ({
      streamingSessions: { ...state.streamingSessions, [sessionId]: false },
    }));

    const [next, ...rest] = useChatStore.getState().queue[sessionId] ?? [];
    if (next) {
      useChatStore.setState((state) => ({
        queue: { ...state.queue, [sessionId]: rest },
      }));
      useChatStore.getState().send(sessionId, next);
    }
  }
}

export function sessionMessages(sessionId: string): ChatMessage[] {
  return useChatStore.getState().messagesBySession[sessionId] ?? [];
}
