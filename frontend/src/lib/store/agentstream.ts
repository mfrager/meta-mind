import { create } from "zustand";

import type { Event } from "@/lib/wire/agentstream.ts";
import { callIdOf, dataOf, isErrorEvent, passOf } from "@/lib/transcript/model.ts";

/** The dock's filter set. */
export type DockFilter = "all" | "turns" | "passes" | "calls" | "tools" | "verification" | "internals" | "errors";

/** The SSE connection state. */
export type Connection = "idle" | "connecting" | "live" | "reconnecting" | "error";

/** The ring size for persisted rows. `lastSeq` is never rewound by the bound. */
export const MAX_RECORDS = 2000;

/** The most paused records buffered before old ones are dropped. */
const MAX_PENDING = 2000;

/** The most streamed characters kept for one call. */
const MAX_DELTA_CHARS = 200_000;

export type AgentstreamState = {
  sessionId: string | null;
  records: Event[];
  deltas: Record<string, string>;
  lastSeq: number;
  connection: Connection;
  error: string | null;
  paused: boolean;
  pending: Event[];
  filter: DockFilter;
  begin: (sessionId: string | null) => void;
  ingest: (incoming: Event[]) => void;
  setConnection: (connection: Connection, error?: string | null) => void;
  setFilter: (filter: DockFilter) => void;
  setPaused: (paused: boolean) => void;
  clear: () => void;
};

export const useAgentstreamStore = create<AgentstreamState>((set) => ({
  sessionId: null,
  records: [],
  deltas: {},
  lastSeq: 0,
  connection: "idle",
  error: null,
  paused: false,
  pending: [],
  filter: "all",
  begin: (sessionId) =>
    set({
      sessionId,
      records: [],
      deltas: {},
      lastSeq: 0,
      connection: sessionId ? "connecting" : "idle",
      error: null,
      paused: false,
      pending: [],
    }),
  ingest: (incoming) =>
    set((state) => {
      let records = state.records;
      let lastSeq = state.lastSeq;
      let pending = state.pending;
      let deltas = state.deltas;
      let changed = false;

      for (const event of incoming) {
        if (state.sessionId && event.sessionId !== state.sessionId) continue;
        if (event.type === "llm.delta") {
          if (state.paused) continue;
          const callId = callIdOf(event);
          const text = typeof dataOf(event).text === "string" ? (dataOf(event).text as string) : "";
          if (!callId || !text) continue;
          const next = `${deltas[callId] ?? ""}${text}`;
          deltas = { ...deltas, [callId]: next.length > MAX_DELTA_CHARS ? next.slice(-MAX_DELTA_CHARS) : next };
          changed = true;
          continue;
        }
        if (event.seq <= lastSeq) continue;
        lastSeq = event.seq;
        changed = true;
        if (state.paused) {
          pending = [...pending, event].slice(-MAX_PENDING);
        } else {
          records = [...records, event].slice(-MAX_RECORDS);
        }
      }

      return changed ? { records, lastSeq, pending, deltas } : state;
    }),
  setConnection: (connection, error = null) => set({ connection, error }),
  setFilter: (filter) => set({ filter }),
  setPaused: (paused) =>
    set((state) => {
      if (paused) return { paused: true };
      const records = [...state.records, ...state.pending].slice(-MAX_RECORDS);
      return { paused: false, pending: [], records };
    }),
  clear: () => set({ records: [], deltas: {}, pending: [] }),
}));

/** Selected records for the dock. */
export function filterRecords(records: Event[], filter: DockFilter): Event[] {
  if (filter === "all") return records;
  return records.filter((event) => matchesFilter(event, filter));
}

export function matchesFilter(event: Event, filter: DockFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "turns":
      return (
        event.type === "session.started" ||
        event.type === "session.ended" ||
        event.type === "turn.started" ||
        event.type === "turn.completed" ||
        event.type === "turn.failed" ||
        event.type === "user.message" ||
        event.type === "agent.message"
      );
    case "passes":
      return event.type.startsWith("pass.");
    case "calls":
      return event.type.startsWith("llm.");
    case "tools":
      return event.type.startsWith("tool.");
    case "verification":
      return event.type.startsWith("verification.");
    case "internals":
      return event.type.startsWith("internal.") || event.type === "prompt.rendered" || event.type === "state.updated";
    case "errors":
      return isErrorEvent(event);
    default:
      return true;
  }
}

export type PassGroup = { pass: string; rows: Event[] };
export type TurnGroup = { turnId: string; passes: PassGroup[] };

/**
 * Group rows Turn → Pass, preserving first-seen order. Records without a pass
 * live under an empty pass key so a turn header can still render them.
 */
export function groupRecords(records: Event[]): TurnGroup[] {
  const turns = new Map<string, Map<string, Event[]>>();
  for (const event of records) {
    const turnId = event.turnId ?? "session";
    let passes = turns.get(turnId);
    if (!passes) {
      passes = new Map();
      turns.set(turnId, passes);
    }
    const pass = passOf(event) ?? "";
    const rows = passes.get(pass);
    if (rows) rows.push(event);
    else passes.set(pass, [event]);
  }
  return [...turns.entries()].map(([turnId, passes]) => ({
    turnId,
    passes: [...passes.entries()].map(([pass, rows]) => ({ pass, rows })),
  }));
}

/** Every record written under one model call, keyed by its call id. */
export function callRecords(records: Event[], callId: string): Event[] {
  return records.filter((event) => callIdOf(event) === callId);
}

/** Tool rows for the same pass attempt and round as a model request. */
export function requestToolRecords(records: Event[], request: Event): Event[] {
  const data = dataOf(request);
  const pass = passOf(request);
  const attempt = typeof data.attempt === "number" ? data.attempt : undefined;
  const round = typeof data.round === "number" ? data.round : undefined;
  return records.filter((event) => {
    if (event.turnId !== request.turnId) return false;
    if (event.type !== "tool.call" && event.type !== "tool.result") return false;
    const tool = dataOf(event);
    if (passOf(event) !== pass) return false;
    if (attempt !== undefined && tool.attempt !== attempt) return false;
    if (round !== undefined && tool.round !== round) return false;
    return true;
  });
}
