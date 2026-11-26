import { create } from "zustand";

import type { StatusFrame, StatusView } from "@/lib/wire/inspector.ts";

export type StatusConnection = "idle" | "loading" | "live" | "reconnecting" | "error";

export type SessionStatusState = {
  view: StatusView | null;
  lastFrameAt: number;
  stale: boolean;
  connection: StatusConnection;
  error: string | null;
};

export const emptySessionStatusState: SessionStatusState = {
  view: null,
  lastFrameAt: 0,
  stale: false,
  connection: "idle",
  error: null,
};

const sectionKeys = [
  "state",
  "session",
  "frames",
  "prompt",
  "budget",
  "data",
  "plans",
  "actions",
  "automation",
  "interjections",
  "agents",
  "operations",
  "unavailable",
] as const;

type SectionKey = (typeof sectionKeys)[number];

function same(left: unknown, right: unknown) {
  return JSON.stringify(left) === JSON.stringify(right);
}

/**
 * Replace only sections whose values changed. A status frame is a full view;
 * keeping stable references for unchanged sections is what prevents a kernel
 * counter from re-rendering the topic header.
 */
export function mergeStatusView(previous: StatusView | null, frame: StatusView): StatusView {
  if (!previous) return frame;
  const next = { ...frame };
  const record = next as unknown as Record<SectionKey, unknown>;
  const old = previous as unknown as Record<SectionKey, unknown>;
  for (const key of sectionKeys) {
    if (same(old[key], record[key])) {
      record[key] = old[key];
    }
  }
  return next;
}

/** Ignore duplicate and stale revisions. */
export function applyStatusFrame(
  state: SessionStatusState,
  frame: StatusFrame,
  now = Date.now(),
): SessionStatusState {
  if (state.view && frame.revision <= state.view.revision) return state;
  return {
    ...state,
    view: mergeStatusView(state.view, frame),
    lastFrameAt: now,
    stale: false,
    connection: "live",
    error: null,
  };
}

/** Frames queued inside one animation frame collapse to the highest revision. */
export function coalesceStatusFrames(frames: StatusFrame[]): StatusFrame | null {
  let latest: StatusFrame | null = null;
  for (const frame of frames) {
    if (!latest || frame.revision > latest.revision) latest = frame;
  }
  return latest;
}

type StatusStore = {
  bySession: Record<string, SessionStatusState>;
  ensure: (sessionId: string) => void;
  begin: (sessionId: string) => void;
  setView: (sessionId: string, view: StatusView, now?: number) => void;
  setConnection: (sessionId: string, connection: StatusConnection, error?: string | null) => void;
  touch: (sessionId: string, now?: number) => void;
  markStale: (sessionId: string) => void;
  clear: (sessionId: string) => void;
};

export const useStatusStore = create<StatusStore>()((set) => ({
  bySession: {},
  ensure: (sessionId) =>
    set((state) =>
      state.bySession[sessionId]
        ? state
        : {
            bySession: {
              ...state.bySession,
              [sessionId]: { ...emptySessionStatusState, connection: "loading" },
            },
          },
    ),
  begin: (sessionId) =>
    set((state) => ({
      bySession: {
        ...state.bySession,
        [sessionId]: {
          ...(state.bySession[sessionId] ?? emptySessionStatusState),
          connection: "loading",
          error: null,
        },
      },
    })),
  setView: (sessionId, view, now = Date.now()) =>
    set((state) => {
      const current = state.bySession[sessionId] ?? emptySessionStatusState;
      const next = applyStatusFrame(current, view, now);
      return next === current ? state : { bySession: { ...state.bySession, [sessionId]: next } };
    }),
  setConnection: (sessionId, connection, error = null) =>
    set((state) => ({
      bySession: {
        ...state.bySession,
        [sessionId]: {
          ...(state.bySession[sessionId] ?? emptySessionStatusState),
          connection,
          error,
        },
      },
    })),
  touch: (sessionId, now = Date.now()) =>
    set((state) => {
      const current = state.bySession[sessionId];
      if (!current) return state;
      if (current.lastFrameAt === now && !current.stale) return state;
      return {
        bySession: {
          ...state.bySession,
          [sessionId]: { ...current, lastFrameAt: now, stale: false, connection: "live" },
        },
      };
    }),
  markStale: (sessionId) =>
    set((state) => {
      const current = state.bySession[sessionId];
      if (!current || current.stale) return state;
      return {
        bySession: {
          ...state.bySession,
          [sessionId]: { ...current, stale: true, connection: "reconnecting" },
        },
      };
    }),
  clear: (sessionId) =>
    set((state) => {
      const bySession = { ...state.bySession };
      delete bySession[sessionId];
      return { bySession };
    }),
}));

export function useSessionStatus(sessionId: string | null): SessionStatusState {
  return useStatusStore((state) =>
    sessionId ? (state.bySession[sessionId] ?? emptySessionStatusState) : emptySessionStatusState,
  );
}

export type { SectionKey };
