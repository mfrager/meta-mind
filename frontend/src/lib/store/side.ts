import { create } from "zustand";

import { fetchDoc } from "@/lib/api/focus.ts";
import type { FocusDoc } from "@/lib/wire/focus.ts";

export type LastAction = { name: string; ok: boolean; summary: string };

type SideState = {
  mode: "state" | "document";
  doc: FocusDoc | null;
  error: string | null;
  revision: number;
  label: string;
  lastAction: LastAction | null;
  showState: () => void;
  showDoc: (doc: FocusDoc) => void;
  setError: (error: string) => void;
  bump: () => void;
  setLastAction: (lastAction: LastAction | null) => void;
};

export const useSideStore = create<SideState>((set) => ({
  mode: "state",
  doc: null,
  error: null,
  revision: 0,
  label: "State",
  lastAction: null,
  showState: () => set({ mode: "state", error: null, label: "State" }),
  showDoc: (doc) => set({ mode: "document", doc, error: null, label: doc.title }),
  setError: (error) => set({ mode: "document", error, label: "Document" }),
  bump: () => set((state) => ({ revision: state.revision + 1 })),
  setLastAction: (lastAction) => set({ lastAction }),
}));

/** A focus frame selects the board or fetches the document the tool named. */
export async function applyControllerFocus(focus: unknown): Promise<void> {
  if (!focus || typeof focus !== "object" || !("id" in focus)) return;
  const id = String((focus as { id: unknown }).id);
  if (id === "state") {
    useSideStore.getState().showState();
    useSideStore.getState().bump();
    return;
  }
  try {
    const doc = await fetchDoc(id);
    useSideStore.getState().showDoc(doc);
  } catch (error) {
    useSideStore.getState().setError(error instanceof Error ? error.message : "The document did not open.");
  }
}
