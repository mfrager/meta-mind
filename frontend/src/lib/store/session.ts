import { create } from "zustand";

type SessionState = {
  id: string | null;
  title: string;
  setSession: (id: string, title: string) => void;
  setTitle: (title: string) => void;
};

export const useSessionStore = create<SessionState>((set) => ({
  id: null,
  title: "Chat",
  setSession: (id, title) => set({ id, title }),
  setTitle: (title) => set({ title }),
}));
