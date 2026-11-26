import { create } from "zustand";

export type PanePropose = {
  name: string;
  input: unknown;
};

type PendingState = {
  text: string | null;
  propose: PanePropose | null;
  queue: (text: string) => void;
  queuePropose: (text: string, propose: PanePropose) => void;
  take: () => string | null;
  takePropose: () => PanePropose | null;
};

export const useChatPending = create<PendingState>((set, get) => ({
  text: null,
  propose: null,
  queue: (text) => set({ text, propose: null }),
  queuePropose: (text, propose) => set({ text, propose }),
  take: () => {
    const text = get().text;
    if (text) set({ text: null });
    return text;
  },
  takePropose: () => {
    const propose = get().propose;
    if (propose) set({ propose: null });
    return propose;
  },
}));
