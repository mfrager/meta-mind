import { create } from "zustand";
import { persist } from "zustand/middleware";

export type ThemeName = "light" | "dark";
export type Density = "comfortable" | "compact";

function initialTheme(): ThemeName {
  if (typeof document === "undefined") return "light";
  return document.documentElement.classList.contains("dark") ? "dark" : "light";
}

type PrefsState = {
  theme: ThemeName;
  density: Density;
  setTheme: (theme: ThemeName) => void;
  toggleTheme: () => void;
  setDensity: (density: Density) => void;
  toggleDensity: () => void;
};

export const usePrefsStore = create<PrefsState>()(
  persist(
    (set) => ({
      theme: initialTheme(),
      density: "comfortable",
      setTheme: (theme) => set({ theme }),
      toggleTheme: () => set((state) => ({ theme: state.theme === "dark" ? "light" : "dark" })),
      setDensity: (density) => set({ density }),
      toggleDensity: () =>
        set((state) => ({ density: state.density === "compact" ? "comfortable" : "compact" })),
    }),
    {
      name: "mm-prefs",
      partialize: (state) => ({ theme: state.theme, density: state.density }),
    },
  ),
);
