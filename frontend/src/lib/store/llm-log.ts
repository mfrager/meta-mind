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

/**
 * One request and everything it caused: its terminal event and, for a retry,
 * the nested call whose `parentCallId` names it. Groups are a projection of
 * the bounded row ring, so grouping never changes what the feed retains.
 */
export type CallGroup = {
  callId: string;
  request: RequestRow;
  outcome: ResponseEvent | ErrorEvent | null;
  children: CallGroup[];
  /** True when the named parent is no longer in the ring (or the link is malformed). */
  orphan?: boolean;
};

export const MAX_ROWS = 500;
export const MAX_STREAM_CHARS = 2_000;

/**
 * Nest outcomes under their request and retries under the request that failed.
 * Nesting is capped at two levels: a deeper chain is flattened onto its oldest
 * ancestor. A parent that was evicted from the ring yields a marked orphan root
 * rather than a dropped row.
 */
export function groupRows(rows: LlmLogRow[]): CallGroup[] {
  const outcomes = new Map<string, ResponseEvent | ErrorEvent>();
  for (const row of rows) {
    if (row.kind === "response" || row.kind === "error") outcomes.set(row.callId, row);
  }

  const groups = new Map<string, CallGroup>();
  for (const row of rows) {
    if (row.kind !== "request" || groups.has(row.callId)) continue;
    groups.set(row.callId, {
      callId: row.callId,
      request: row,
      outcome: outcomes.get(row.callId) ?? null,
      children: [],
    });
  }

  const roots: CallGroup[] = [];
  for (const group of groups.values()) {
    const parentId = group.request.parentCallId ?? null;
    let parent: CallGroup | undefined;
    let orphan = false;

    if (parentId) {
      const seen = new Set<string>([group.callId]);
      let cursor: CallGroup | undefined = groups.get(parentId);
      while (cursor) {
        if (seen.has(cursor.callId)) {
          // A cyclic or self-referential parent link is malformed, not a tree.
          cursor = undefined;
          orphan = true;
          break;
        }
        seen.add(cursor.callId);
        const nextId = cursor.request.parentCallId ?? null;
        const next = nextId ? groups.get(nextId) : undefined;
        if (!next) break;
        cursor = next;
      }
      parent = cursor;
      if (!parent) orphan = true;
    }

    if (parent) {
      parent.children.push(group);
    } else {
      if (orphan) group.orphan = true;
      roots.push(group);
    }
  }

  return roots;
}

function hasOutcome(group: CallGroup, kind: "response" | "error"): boolean {
  if (group.outcome?.kind === kind) return true;
  return group.children.some((child) => hasOutcome(child, kind));
}

/** Filter groups, never separating an outcome from the request that caused it. */
export function filterGroups(groups: CallGroup[], filter: LlmLogFilter): CallGroup[] {
  switch (filter) {
    case "requests":
      return groups.map((group) => ({ ...group, outcome: null, children: [] }));
    case "streams":
      return groups.filter((group) => group.request.streamed);
    case "responses":
      return groups.filter((group) => hasOutcome(group, "response"));
    case "errors":
      return groups.filter((group) => hasOutcome(group, "error"));
    case "all":
      return groups;
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
