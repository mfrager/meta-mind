import { useEffect, useMemo, useRef, useState } from "react";

import { modLabel, type Shortcut } from "@/lib/shortcuts.ts";
import { usePaneStore } from "@/lib/store/pane.ts";

type Row = {
  id: Shortcut | "tree" | "map";
  keys: string;
  action: string;
  note?: string;
};

export function ShortcutDialog() {
  const open = usePaneStore((state) => state.shortcutsOpen);
  const setOpen = usePaneStore((state) => state.setShortcutsOpen);
  const ref = useRef<HTMLDialogElement>(null);
  const [query, setQuery] = useState("");
  const mod = modLabel();

  const rows = useMemo<Row[]>(
    () => [
      { id: "palette", keys: `${mod}+K`, action: "Focus the composer" },
      { id: "composer", keys: `${mod}+L`, action: "Focus the composer" },
      { id: "document", keys: `${mod}+1`, action: "Focus the document pane" },
      { id: "chat", keys: `${mod}+2`, action: "Focus the chat pane" },
      { id: "swap", keys: `${mod}+\\`, action: "Swap the panes" },
      { id: "sidebar", keys: `${mod}+B`, action: "Toggle the sidebar" },
      { id: "fullscreen", keys: `${mod}+Shift+F`, action: "Fullscreen the active pane" },
      { id: "mode", keys: `${mod}+E`, action: "Toggle view and edit" },
      { id: "save", keys: `${mod}+S`, action: "Save a draft, or send from the composer" },
      { id: "cheatsheet", keys: `${mod}+/`, action: "This cheatsheet" },
      { id: "dismiss", keys: "Esc", action: "Close the overlay, then the notice, then fullscreen" },
      {
        id: "quick-switch",
        keys: `${mod}+G`,
        action: "Document quick-switcher",
        note: "Document tabs arrive with the document pane.",
      },
      {
        id: "duplicate",
        keys: `${mod}+D`,
        action: "Duplicate the document",
        note: "Document tabs arrive with the document pane.",
      },
      {
        id: "close-tab",
        keys: `${mod}+W`,
        action: "Close the tab",
        note: "Document tabs arrive with the document pane.",
      },
      { id: "tree", keys: "J / K", action: "Move in a tree", note: "Arrives with the tree editors." },
      { id: "map", keys: "F", action: "Zoom the mind map to fit", note: "Arrives with the mind map." },
    ],
    [mod],
  );

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setQuery("");
      dialog.showModal();
    }
    if (!open && dialog.open) dialog.close();
  }, [open]);

  const filtered = rows.filter((row) => {
    const haystack = `${row.keys} ${row.action} ${row.note ?? ""}`.toLowerCase();
    return haystack.includes(query.trim().toLowerCase());
  });

  return (
    <dialog
      ref={ref}
      className="mm-dialog"
      aria-label="Keyboard shortcuts"
      onClose={() => setOpen(false)}
    >
      <form method="dialog" className="border-border flex items-center gap-2 border-b px-3">
        <label className="sr-only" htmlFor="shortcut-search">
          Search shortcuts
        </label>
        <input
          id="shortcut-search"
          value={query}
          placeholder="Search shortcuts…"
          className="text-body h-11 w-full bg-transparent outline-none"
          onChange={(event) => setQuery(event.target.value)}
        />
        <button type="submit" className="text-small text-muted-foreground">
          Close
        </button>
      </form>
      <ul className="pane-scroll max-h-80 overflow-auto p-2">
        {filtered.map((row) => (
          <li key={row.id} className="flex items-baseline gap-3 px-2 py-1">
            <kbd className="border-border w-28 shrink-0 rounded-xs border px-1 font-mono text-value">{row.keys}</kbd>
            <span className="text-body min-w-0">
              {row.action}
              {row.note ? <span className="text-small text-muted-foreground mt-0.5 block">{row.note}</span> : null}
            </span>
          </li>
        ))}
        {filtered.length === 0 ? (
          <li className="text-small text-muted-foreground px-2 py-3">No shortcut matches.</li>
        ) : null}
      </ul>
    </dialog>
  );
}
