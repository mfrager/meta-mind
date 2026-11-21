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

function applyEvent(message: ChatMessage, event: AgentEvent): ChatMessage {
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
    case "tool":
      return { ...message, parts: upsertToolStep(message.parts, event) };
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
        parts: markThinkingDone(message.parts),
        usage: event.usage ?? message.usage,
      };
    case "error":
      return {
        ...message,
        status: "error",
        error: event.message,
        parts: markThinkingDone(message.parts),
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
