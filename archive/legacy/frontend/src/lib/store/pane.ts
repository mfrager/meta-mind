import { create } from "zustand";
import { persist } from "zustand/middleware";

export type PaneId = "document" | "chat";

type PaneState = {
  swapped: boolean;
  documentCollapsed: boolean;
  chatCollapsed: boolean;
  sidebarCollapsed: boolean;
  fullscreen: boolean;
  activePane: PaneId;
  documentSize: number;
  chatSize: number;
  /** Bumped to remount the split after a reset. Not persisted. */
  layoutEpoch: number;
  paletteOpen: boolean;
  shortcutsOpen: boolean;
  sidebarSheet: boolean;
  toggleSwap: () => void;
  toggleSidebar: () => void;
  toggleSheet: () => void;
  setSheet: (open: boolean) => void;
  toggleCollapsed: (pane: PaneId) => void;
  expand: (pane: PaneId) => void;
  toggleFullscreen: () => void;
  setActivePane: (pane: PaneId) => void;
  setSizes: (documentSize: number, chatSize: number) => void;
  resetLayout: () => void;
  setPaletteOpen: (open: boolean) => void;
  setShortcutsOpen: (open: boolean) => void;
};

export const usePaneStore = create<PaneState>()(
  persist(
    (set, get) => ({
      swapped: false,
      documentCollapsed: false,
      chatCollapsed: false,
      sidebarCollapsed: false,
      fullscreen: false,
      activePane: "chat",
      documentSize: 42,
      chatSize: 58,
      layoutEpoch: 0,
      paletteOpen: false,
      shortcutsOpen: false,
      sidebarSheet: false,
      toggleSwap: () => set((state) => ({ swapped: !state.swapped })),
      toggleSidebar: () => set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),
      toggleSheet: () => set((state) => ({ sidebarSheet: !state.sidebarSheet })),
      setSheet: (sidebarSheet) => set({ sidebarSheet }),
      toggleCollapsed: (pane) =>
        set((state) => {
          if (pane === "chat") return {};
          return {
            documentCollapsed: !state.documentCollapsed,
            activePane: state.documentCollapsed ? "document" : state.activePane,
          };
        }),
      expand: (pane) =>
        set(
          pane === "document"
            ? { documentCollapsed: false, activePane: "document" }
            : { chatCollapsed: false, activePane: "chat" },
        ),
      toggleFullscreen: () => {
        const state = get();
        if (state.fullscreen) {
          set({ fullscreen: false });
          return;
        }
        set({
          fullscreen: true,
          documentCollapsed: state.activePane === "document" ? false : state.documentCollapsed,
          chatCollapsed: state.activePane === "chat" ? false : state.chatCollapsed,
        });
      },
      setActivePane: (activePane) => set({ activePane }),
      setSizes: (documentSize, chatSize) => set({ documentSize, chatSize }),
      resetLayout: () =>
        set((state) => ({
          swapped: false,
          documentCollapsed: false,
          chatCollapsed: false,
          fullscreen: false,
          documentSize: 42,
          chatSize: 58,
          layoutEpoch: state.layoutEpoch + 1,
        })),
      setPaletteOpen: (paletteOpen) =>
        set({ paletteOpen, shortcutsOpen: paletteOpen ? false : get().shortcutsOpen }),
      setShortcutsOpen: (shortcutsOpen) =>
        set({ shortcutsOpen, paletteOpen: shortcutsOpen ? false : get().paletteOpen }),
    }),
    {
      name: "mm-pane-chat",
      partialize: (state) => ({
        swapped: state.swapped,
        documentCollapsed: state.documentCollapsed,
        chatCollapsed: state.chatCollapsed,
        sidebarCollapsed: state.sidebarCollapsed,
        fullscreen: state.fullscreen,
        activePane: state.activePane,
        documentSize: state.documentSize,
        chatSize: state.chatSize,
      }),
    },
  ),
);
