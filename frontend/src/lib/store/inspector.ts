import { create } from "zustand";
import { persist } from "zustand/middleware";

export type InspectorTab = "status" | "knowledge" | "plan" | "trace";

type InspectorState = {
  open: boolean;
  width: number;
  tab: InspectorTab;
  selectedTrace: string | null;
  setOpen: (open: boolean) => void;
  toggle: () => void;
  setWidth: (width: number) => void;
  setTab: (tab: InspectorTab) => void;
  setSelectedTrace: (traceId: string | null) => void;
};

export const useInspectorStore = create<InspectorState>()(
  persist(
    (set) => ({
      open: true,
      width: 340,
      tab: "status",
      selectedTrace: null,
      setOpen: (open) => set({ open }),
      toggle: () => set((state) => ({ open: !state.open })),
      setWidth: (width) => set({ width: Math.min(480, Math.max(280, width)) }),
      setTab: (tab) => set({ tab }),
      setSelectedTrace: (selectedTrace) => set({ selectedTrace }),
    }),
    {
      name: "mm-inspector",
      partialize: (state) => ({
        open: state.open,
        width: state.width,
        tab: state.tab,
      }),
    },
  ),
);

export const inspectorTabs: { id: InspectorTab; label: string }[] = [
  { id: "status", label: "Status" },
  { id: "knowledge", label: "Knowledge" },
  { id: "plan", label: "Plan" },
  { id: "trace", label: "Trace" },
];
