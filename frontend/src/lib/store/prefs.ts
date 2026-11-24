import { create } from "zustand";
import { persist } from "zustand/middleware";

export type ThemeName = "light" | "dark";
export type Density = "comfortable" | "compact";

export const LLM_LOG_MIN_HEIGHT = 96;
export const LLM_LOG_DEFAULT_HEIGHT = 168;
export const LLM_LOG_MAX_HEIGHT = 400;

export function clampLlmLogHeight(height: number): number {
  return Math.min(LLM_LOG_MAX_HEIGHT, Math.max(LLM_LOG_MIN_HEIGHT, Math.round(height)));
}

function initialTheme(): ThemeName {
  if (typeof document === "undefined") return "light";
  return document.documentElement.classList.contains("dark") ? "dark" : "light";
}

type PrefsState = {
  theme: ThemeName;
  density: Density;
  llmLogCollapsed: boolean;
  llmLogHeight: number;
  setTheme: (theme: ThemeName) => void;
  toggleTheme: () => void;
  setDensity: (density: Density) => void;
  toggleDensity: () => void;
  setLlmLogCollapsed: (collapsed: boolean) => void;
  toggleLlmLogCollapsed: () => void;
  setLlmLogHeight: (height: number) => void;
};

export const usePrefsStore = create<PrefsState>()(
  persist(
    (set) => ({
      theme: initialTheme(),
      density: "comfortable",
      llmLogCollapsed: false,
      llmLogHeight: LLM_LOG_DEFAULT_HEIGHT,
      setTheme: (theme) => set({ theme }),
      toggleTheme: () => set((state) => ({ theme: state.theme === "dark" ? "light" : "dark" })),
      setDensity: (density) => set({ density }),
      toggleDensity: () => set((state) => ({ density: state.density === "compact" ? "comfortable" : "compact" })),
      setLlmLogCollapsed: (llmLogCollapsed) => set({ llmLogCollapsed }),
      toggleLlmLogCollapsed: () => set((state) => ({ llmLogCollapsed: !state.llmLogCollapsed })),
      setLlmLogHeight: (height) => set({ llmLogHeight: clampLlmLogHeight(height) }),
    }),
    {
      name: "mm-prefs",
      partialize: (state) => ({
        theme: state.theme,
        density: state.density,
        llmLogCollapsed: state.llmLogCollapsed,
        llmLogHeight: state.llmLogHeight,
      }),
    },
  ),
);
