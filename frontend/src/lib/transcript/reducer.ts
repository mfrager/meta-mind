import type { ChatFrame, ChatRunState, Phase } from "@/lib/wire/chat.ts";

export type OpCall = {
  id: string;
  name: string;
  phase: Phase;
  input: unknown;
  output?: unknown;
  success?: boolean;
  durationMs?: number;
};

export type ButtonOption = { label: string; value: string; description?: string };

export type ButtonRequest = {
  kind: "buttons" | "form";
  prompt?: string;
  columns?: number;
  variant?: string;
  options: ButtonOption[];
  fields?: { name: string; label: string }[];
};

export type TranscriptItem =
  | { kind: "user"; id: string; content: string }
  | { kind: "turn"; id: string; phase: Phase; text: string; state: "processing" | "complete" | "error"; visible: boolean; ops: OpCall[] }
  | { kind: "input_request"; id: string; request: ButtonRequest; answered?: string }
  | { kind: "system"; id: string; level: "info" | "error"; message: string; reason?: string }
  | { kind: "final"; id: string; text: string; suggestions: string[] };

export type TranscriptState = {
  items: TranscriptItem[];
  seen: number[];
  run: ChatRunState | "idle";
  requestId: string | null;
  sessionId: string | null;
  lastSeq: number;
  focus: unknown;
};

export const initialTranscript: TranscriptState = {
  items: [],
  seen: [],
  run: "idle",
  requestId: null,
  sessionId: null,
  lastSeq: 0,
  focus: null,
};

export type SequencedFrame = { seq: number; frame: ChatFrame };

/** Apply one numbered frame. A repeated sequence in the same run is a no-op so resume cannot duplicate. */
export function reduceTranscript(state: TranscriptState, sequenced: SequencedFrame): TranscriptState {
  const nextRun = sequenced.frame.type === "started" && (state.run === "completed" || state.run === "failed" || state.run === "idle");
  const base = nextRun ? { ...state, seen: [], lastSeq: 0 } : state;
  if (base.seen.includes(sequenced.seq)) return state;
  const next = apply(base, sequenced.frame);
  return {
    ...next,
    seen: [...base.seen, sequenced.seq],
    lastSeq: Math.max(base.lastSeq, sequenced.seq),
  };
}

export function reduceFrames(state: TranscriptState, frames: SequencedFrame[]): TranscriptState {
  return frames.reduce(reduceTranscript, state);
}

function apply(state: TranscriptState, frame: ChatFrame): TranscriptState {
  switch (frame.type) {
    case "started":
      return state;
    case "run":
      return {
        ...state,
        run: "processing",
        requestId: frame.requestId,
        sessionId: frame.sessionId,
        items: state.items.some((item) => item.kind === "user" && item.id === frame.requestId)
          ? state.items
          : [...state.items, { kind: "user", id: frame.requestId, content: frame.query }],
      };
    case "reconnecting":
      return {
        ...state,
        run: "reconnecting",
        items: [
          ...state.items,
          {
            kind: "system",
            id: `reconnect-${frame.attempt}`,
            level: "info",
            message: `Reconnecting to the backend… attempt ${frame.attempt}`,
          },
        ],
      };
    case "llm_request_started":
      return {
        ...state,
        items: upsertTurn(state.items, frame.turn_id, {
          phase: frame.phase,
          visible: frame.visible,
          state: "processing",
        }),
      };
    case "llm_response":
      return {
        ...state,
        items: upsertTurn(state.items, frame.turn_id, {
          phase: frame.phase,
          visible: frame.visible,
          text: textOf(state.items, frame.turn_id) + frame.text,
        }),
      };
    case "llm_request_finished":
      return {
        ...state,
        items: upsertTurn(state.items, frame.turn_id, {
          state: frame.outcome === "error" || frame.outcome === "cancelled" ? "error" : "complete",
        }),
      };
    case "tool_call":
      return { ...state, items: addOp(state.items, frame.turn_id, frame) };
    case "tool_result":
      return { ...state, items: fillOp(state.items, frame) };
    case "input_request":
      return {
        ...state,
        items: [
          ...state.items,
          { kind: "input_request", id: frame.id, request: asButtons(frame.request) },
        ],
      };
    case "input_answer":
      return {
        ...state,
        items: state.items.map((item) =>
          item.kind === "input_request" && item.id === frame.id
            ? { ...item, answered: typeof frame.answer === "string" ? frame.answer : JSON.stringify(frame.answer) }
            : item,
        ),
      };
    case "final_response":
      return {
        ...state,
        items: [
          ...state.items,
          {
            kind: "final",
            id: frame.response_id,
            text: frame.text,
            suggestions: frame.suggestions ?? [],
          },
        ],
      };
    case "failed":
      return {
        ...state,
        run: "failed",
        items: [
          ...state.items,
          {
            kind: "system",
            id: `failed-${state.lastSeq}`,
            level: "error",
            message: frame.error,
            reason: frame.reason ?? undefined,
          },
        ],
      };
    case "completed":
      return { ...state, run: "completed" };
    case "focus":
      return { ...state, focus: frame.doc };
    case "progress":
      return {
        ...state,
        items: [
          ...state.items,
          { kind: "system", id: `progress-${frame.label}`, level: "info", message: `${frame.label} ${frame.value}/${frame.max}` },
        ],
      };
    case "status":
      return {
        ...state,
        items: [
          ...state.items,
          {
            kind: "system",
            id: `status-${frame.status}`,
            level: "info",
            message: frame.status,
            reason: frame.reason ?? undefined,
          },
        ],
      };
    case "block":
      return state;
    default:
      return state;
  }
}

function upsertTurn(
  items: TranscriptItem[],
  id: string,
  patch: Partial<Extract<TranscriptItem, { kind: "turn" }>>,
): TranscriptItem[] {
  const index = items.findIndex((item) => item.kind === "turn" && item.id === id);
  if (index < 0) {
    return [
      ...items,
      {
        kind: "turn",
        id,
        phase: patch.phase ?? "plan",
        text: patch.text ?? "",
        state: patch.state ?? "processing",
        visible: patch.visible ?? true,
        ops: patch.ops ?? [],
      },
    ];
  }
  return items.map((item, itemIndex) => (itemIndex === index && item.kind === "turn" ? { ...item, ...patch } : item));
}

function textOf(items: TranscriptItem[], id: string) {
  const turn = items.find((item) => item.kind === "turn" && item.id === id);
  return turn && turn.kind === "turn" ? turn.text : "";
}

function addOp(items: TranscriptItem[], turnId: string, frame: Extract<ChatFrame, { type: "tool_call" }>): TranscriptItem[] {
  const withTurn = upsertTurn(items, turnId, { phase: frame.phase });
  return withTurn.map((item) => {
    if (item.kind !== "turn" || item.id !== turnId) return item;
    if (item.ops.some((op) => op.id === frame.id)) return item;
    return {
      ...item,
      ops: [...item.ops, { id: frame.id, name: frame.name, phase: frame.phase, input: frame.input }],
    };
  });
}

function fillOp(items: TranscriptItem[], frame: Extract<ChatFrame, { type: "tool_result" }>): TranscriptItem[] {
  return items.map((item) => {
    if (item.kind !== "turn") return item;
    return {
      ...item,
      ops: item.ops.map((op) =>
        op.id === frame.id ? { ...op, output: frame.output, success: frame.success, durationMs: frame.durationMs } : op,
      ),
    };
  });
}

function asButtons(value: unknown): ButtonRequest {
  const record = value && typeof value === "object" ? (value as Record<string, unknown>) : {};
  const options = Array.isArray(record.options)
    ? record.options.flatMap((option) => {
        if (!option || typeof option !== "object") return [];
        const row = option as Record<string, unknown>;
        if (typeof row.label !== "string" || typeof row.value !== "string") return [];
        return [{ label: row.label, value: row.value, description: typeof row.description === "string" ? row.description : undefined }];
      })
    : [];
  const fields = Array.isArray(record.fields)
    ? record.fields.flatMap((field) => {
        if (!field || typeof field !== "object") return [];
        const row = field as Record<string, unknown>;
        if (typeof row.name !== "string" || typeof row.label !== "string") return [];
        return [{ name: row.name, label: row.label }];
      })
    : [];
  return {
    kind: record.kind === "form" ? "form" : "buttons",
    prompt: typeof record.prompt === "string" ? record.prompt : undefined,
    columns: typeof record.columns === "number" ? record.columns : 2,
    variant: typeof record.variant === "string" ? record.variant : "options",
    options,
    fields,
  };
}
