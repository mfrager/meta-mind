import { useSyncExternalStore } from "react";

export type StageTick = { docId: string; reason: string };

let ticks: StageTick[] = [];
const listeners = new Set<() => void>();

export function recordStageTick(tick: StageTick) {
  if (ticks.some((item) => item.docId === tick.docId && item.reason === tick.reason)) return;
  ticks = [...ticks, tick];
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useStageTicks(docId: string): StageTick[] {
  const all = useSyncExternalStore(subscribe, () => ticks, () => ticks);
  return all.filter((tick) => tick.docId === docId);
}
