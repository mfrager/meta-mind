import { create } from "zustand";

type PendingState = {
  text: string | null;
  queue: (text: string) => void;
  take: () => string | null;
};

export const useChatPending = create<PendingState>((set, get) => ({
  text: null,
  queue: (text) => set({ text }),
  take: () => {
    const text = get().text;
    if (text) set({ text: null });
    return text;
  },
}));
