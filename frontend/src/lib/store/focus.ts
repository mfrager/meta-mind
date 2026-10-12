import { create } from "zustand";
import { persist } from "zustand/middleware";

export type DocMode = "view" | "edit";

type FocusState = {
  mode: DocMode;
  notice: string | null;
  toggleMode: () => void;
  setMode: (mode: DocMode) => void;
  setNotice: (notice: string) => void;
  clearNotice: () => void;
};

export const useFocusStore = create<FocusState>()(
  persist(
    (set) => ({
      mode: "view",
      notice: null,
      toggleMode: () => set((state) => ({ mode: state.mode === "view" ? "edit" : "view" })),
      setMode: (mode) => set({ mode }),
      setNotice: (notice) => set({ notice }),
      clearNotice: () => set({ notice: null }),
    }),
    {
      name: "mm-focus",
      partialize: (state) => ({ mode: state.mode }),
    },
  ),
);
