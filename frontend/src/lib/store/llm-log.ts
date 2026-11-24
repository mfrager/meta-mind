import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { LlmActivityEvent, LlmActivitySnapshot } from "@/lib/wire/llm_activity.ts";

type RequestEvent = Extract<LlmActivityEvent, { kind: "request" }>;
type DeltaEvent = Extract<LlmActivityEvent, { kind: "delta" }>;
type ResponseEvent = Extract<LlmActivityEvent, { kind: "response" }>;
type ErrorEvent = Extract<LlmActivityEvent, { kind: "error" }>;

export type RequestRow = RequestEvent & {
  streamText: string;
  streamChars: number;
  streamTruncated: boolean;
};

export type LlmLogRow = RequestRow | DeltaEvent | ResponseEvent | ErrorEvent;

export type LlmLogConnection = "idle" | "loading" | "connecting" | "live" | "reconnecting" | "error";
export type LlmLogFilter = "all" | "requests" | "streams" | "responses" | "errors";

export const MAX_ROWS = 500;
export const MAX_STREAM_CHARS = 2_000;

export function filterRows(rows: LlmLogRow[], filter: LlmLogFilter): LlmLogRow[] {
  switch (filter) {
    case "requests":
      return rows.filter((row) => row.kind === "request");
    case "streams":
      return rows.filter((row) => row.kind === "delta" || (row.kind === "request" && row.streamed));
    case "responses":
      return rows.filter((row) => row.kind === "response");
    case "errors":
      return rows.filter((row) => row.kind === "error");
    case "all":
      return rows;
  }
}

export type LlmLogState = {
  rows: LlmLogRow[];
  buffer: LlmLogRow[];
  oldestSeq: number | null;
  lastSeq: number;
  hydrated: boolean;
  connection: LlmLogConnection;
  error: string | null;
  paused: boolean;
  pausedCount: number;
  filter: LlmLogFilter;
};

export const initialLlmLogState: LlmLogState = {
  rows: [],
  buffer: [],
  oldestSeq: null,
  lastSeq: 0,
  hydrated: false,
  connection: "idle",
  error: null,
  paused: false,
  pausedCount: 0,
  filter: "all",
};

function bound(rows: LlmLogRow[]): LlmLogRow[] {
  return rows.length > MAX_ROWS ? rows.slice(rows.length - MAX_ROWS) : rows;
}

function capStreamText(text: string): string {
  if (text.length <= MAX_STREAM_CHARS) return text;
  return text.slice(0, MAX_STREAM_CHARS);
}

/** Append a stream fragment to its request row; a late fragment stays a delta. */
function mergeDelta(rows: LlmLogRow[], event: DeltaEvent): LlmLogRow[] {
  for (let index = rows.length - 1; index >= 0; index -= 1) {
    const row = rows[index];
    if (!row || row.kind !== "request" || row.callId !== event.callId) continue;
    const next = [...rows];
    next[index] = {
      ...row,
      streamText: capStreamText(row.streamText + event.text),
      streamChars: event.chars,
      streamTruncated: row.streamTruncated || event.truncated,
    };
    return next;
  }
  return [...rows, event];
}

/** Apply one ordered event to a row list, folding deltas into their request. */
export function applyActivityEvent(rows: LlmLogRow[], event: LlmActivityEvent): LlmLogRow[] {
  if (event.kind === "delta") return bound(mergeDelta(rows, event));
  if (event.kind === "request") {
    return bound([...rows, { ...event, streamText: "", streamChars: 0, streamTruncated: false }]);
  }
  return bound([...rows, event]);
}

export function rowsFromEvents(events: LlmActivityEvent[]): LlmLogRow[] {
  let rows: LlmLogRow[] = [];
  for (const event of events) rows = applyActivityEvent(rows, event);
  return rows;
}

type LlmLogActions = {
  /** Ignore duplicates and stale cursors, then fold the event into the feed. */
  ingest: (seq: number, event: LlmActivityEvent) => void;
  /** Replace the visible ring from a snapshot or resync payload. */
  resync: (snapshot: Pick<LlmActivitySnapshot, "oldestSeq" | "nextSeq" | "events">) => void;
  resyncEvents: (snapshot: { oldestSeq: number; events: LlmActivityEvent[] }) => void;
  setConnection: (connection: LlmLogConnection, error?: string | null) => void;
  setPaused: (paused: boolean) => void;
  setFilter: (filter: LlmLogFilter) => void;
  clear: () => void;
  reset: () => void;
};

export const useLlmLogStore = create<LlmLogState & LlmLogActions>()(
  persist(
    (set) => ({
      ...initialLlmLogState,
      ingest: (seq, event) =>
        set((state) => {
          if (seq <= state.lastSeq) return state;
          if (state.paused) {
            const buffer = state.buffer.length > 0 ? state.buffer : state.rows;
            return {
              ...state,
              lastSeq: seq,
              buffer: applyActivityEvent(buffer, event),
              pausedCount: state.pausedCount + 1,
            };
          }
          return {
            ...state,
            lastSeq: seq,
            rows: applyActivityEvent(state.rows, event),
          };
        }),
      resync: (snapshot) =>
        set((state) => ({
          ...state,
          rows: rowsFromEvents(snapshot.events),
          buffer: [],
          oldestSeq: snapshot.oldestSeq,
          lastSeq: snapshot.nextSeq - 1,
          hydrated: true,
          error: null,
          pausedCount: 0,
        })),
      resyncEvents: (snapshot) =>
        set((state) => {
          const events = [...snapshot.events].sort((left, right) => left.seq - right.seq);
          const highest = events.reduce((seq, event) => Math.max(seq, event.seq), 0);
          return {
            ...state,
            rows: rowsFromEvents(events),
            buffer: [],
            oldestSeq: snapshot.oldestSeq,
            lastSeq: Math.max(state.lastSeq, highest),
            hydrated: true,
            error: null,
            pausedCount: 0,
          };
        }),
      setConnection: (connection, error = null) => set((state) => ({ ...state, connection, error })),
      setPaused: (paused) =>
        set((state) => {
          if (paused === state.paused) return state;
          if (paused) {
            return { ...state, paused: true, buffer: [], pausedCount: 0 };
          }
          return {
            ...state,
            paused: false,
            rows: state.buffer.length > 0 ? state.buffer : state.rows,
            buffer: [],
            pausedCount: 0,
          };
        }),
      clear: () => set((state) => ({ ...state, rows: [], buffer: [], pausedCount: 0 })),
      reset: () => set({ ...initialLlmLogState }),
      setFilter: (filter) => set((state) => ({ ...state, filter })),
    }),
    {
      name: "mm-llm-log",
      partialize: (state) => ({ filter: state.filter }),
    },
  ),
);

export function useLlmLog() {
  return useLlmLogStore((state) => state);
}
