import { create } from "zustand";

import { applyDocFrame, type FocusFrame } from "@/lib/focus/apply.ts";
import type { FocusDoc } from "@/lib/wire/focus.ts";

type DocumentsState = {
  docs: FocusDoc[];
  activeId: string | null;
  historyOpen: boolean;
  lineageOpen: boolean;
  wizardOpen: boolean;
  open: (doc: FocusDoc) => void;
  applyFrame: (frame: FocusFrame) => void;
  close: (id: string) => void;
  cycle: () => void;
  setHistory: (open: boolean) => void;
  setLineage: (open: boolean) => void;
  setWizard: (open: boolean) => void;
};

export const useDocuments = create<DocumentsState>((set) => ({
  docs: [],
  activeId: null,
  historyOpen: false,
  lineageOpen: false,
  wizardOpen: false,
  open: (doc) =>
    set((state) => ({
      docs: state.docs.some((item) => item.id === doc.id)
        ? state.docs.map((item) => (item.id === doc.id ? doc : item))
        : [...state.docs, doc],
      activeId: doc.id,
    })),
  applyFrame: (frame) =>
    set((state) => {
      const next = applyDocFrame(state.docs, frame);
      return { docs: next.docs, activeId: next.openedId };
    }),
  close: (id) =>
    set((state) => {
      const docs = state.docs.filter((item) => item.id !== id);
      return { docs, activeId: state.activeId === id ? (docs.at(-1)?.id ?? null) : state.activeId };
    }),
  cycle: () =>
    set((state) => {
      if (state.docs.length === 0) return state;
      const index = Math.max(0, state.docs.findIndex((item) => item.id === state.activeId));
      return { activeId: state.docs[(index + 1) % state.docs.length]?.id ?? null };
    }),
  setHistory: (historyOpen) => set({ historyOpen }),
  setLineage: (lineageOpen) => set({ lineageOpen }),
  setWizard: (wizardOpen) => set({ wizardOpen }),
}));

export function activeDocument(state: DocumentsState): FocusDoc | null {
  return state.docs.find((item) => item.id === state.activeId) ?? null;
}
