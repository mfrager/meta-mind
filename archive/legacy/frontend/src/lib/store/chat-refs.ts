import { create } from "zustand";

type RefState = {
  refs: string[];
  add: (ref: string) => void;
  clear: () => void;
};

export const useChatRefs = create<RefState>((set) => ({
  refs: [],
  add: (ref) => set((state) => (state.refs.includes(ref) ? state : { refs: [...state.refs, ref] })),
  clear: () => set({ refs: [] }),
}));
