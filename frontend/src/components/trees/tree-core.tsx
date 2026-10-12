import { type KeyboardEvent, useState } from "react";
import { Link } from "react-router-dom";

import { OutlineView } from "@/components/trees/outline.tsx";
import { SunburstView } from "@/components/trees/sunburst.tsx";
import { TreemapView } from "@/components/trees/treemap.tsx";
import { TreeTableView } from "@/components/trees/tree-table.tsx";
import { useTreeController } from "@/components/trees/use-tree-controller.ts";
import { treeMapRef, visibleRows, type TreeAction } from "@/lib/trees/model.ts";

type Lens = "outline" | "table" | "sunburst" | "treemap";

export function TreeCore({ name, readOnly = false }: { name: string; readOnly?: boolean }) {
  const { snapshot, state, error, eventId, dispatch, commit } = useTreeController(name);
  const commitView = (action: TreeAction) => {
    if (readOnly && action.type !== "focus" && action.type !== "toggle" && action.type !== "search") {
      return;
    }
    commit(action);
  };
  const [lens, setLens] = useState<Lens>("outline");
  const [chatRef, setChatRef] = useState("");
  const rows = visibleRows(state.nodes, new Set(state.expanded), state.query);
  const focus = state.nodes.find((node) => node.id === state.focusId) ?? null;
  const pending = state.nodes.find((node) => node.id === state.pendingDeleteId) ?? null;

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 p-4" data-tree-name={name} onKeyDown={readOnly ? undefined : (event) => onTreeKey(event, commitView)}>
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-h2">{snapshot?.title ?? name}</h2>
        {readOnly ? null : <Link to={`/map/${name}`} className="text-small underline">Open as mind map</Link>}
        <button
          type="button"
          className="text-small underline"
          onClick={() => {
            if (focus) setChatRef(treeMapRef(name, focus.id));
          }}
        >
          Send to chat
        </button>
        {readOnly ? null : (
          <>
            <button type="button" className="text-small" onClick={() => commitView({ type: "undo" })}>Undo</button>
            <button type="button" className="text-small" onClick={() => commitView({ type: "redo" })}>Redo</button>
          </>
        )}
        {(["outline", "table", "sunburst", "treemap"] as const).map((item) => (
          <button key={item} type="button" aria-pressed={lens === item} className="text-small" onClick={() => setLens(item)}>
            {item}
          </button>
        ))}
      </div>
      {snapshot?.reason ? <p data-tree-reason className="text-body">{snapshot.reason}</p> : null}
      {error ? <p className="text-body text-red-700 dark:text-red-300">{error}</p> : null}
      {eventId ? <p className="font-mono text-value" data-tree-event>{eventId}</p> : null}
      {chatRef ? <p className="font-mono text-value" data-chat-ref>{chatRef}</p> : null}
      <input
        aria-label="Filter tree"
        value={state.query}
        className="border-border h-[var(--control-h)] max-w-xs rounded-xs border px-2"
        onChange={(event) => dispatch({ type: "search", query: event.target.value })}
      />
      {state.editingId ? (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const data = new FormData(event.currentTarget);
            const label = String(data.get("label") ?? "");
            if (state.editingId) commitView({ type: "rename", id: state.editingId, label });
          }}
        >
          <input name="label" aria-label="Rename" defaultValue={focus?.label ?? ""} autoFocus className="border-border h-[var(--control-h)] rounded-xs border px-2" />
        </form>
      ) : null}
      {pending ? (
        <div role="alertdialog" aria-label={`Delete ${pending.label}`} className="border-border rounded-xs border p-3">
          <p className="text-body">Delete {pending.label}? Undo restores it.</p>
          <button type="button" onClick={() => commitView({ type: "remove", id: pending.id })}>Delete</button>
          <button type="button" onClick={() => dispatch({ type: "cancel-remove" })}>Cancel</button>
        </div>
      ) : null}
      <div className="min-h-0 flex-1" tabIndex={0}>
        {lens === "outline" ? (
          <OutlineView
            rows={rows}
            nodes={state.nodes}
            focusId={state.focusId}
            expanded={new Set(state.expanded)}
            onFocus={(id) => dispatch({ type: "focus", id })}
            onToggle={(id) => dispatch({ type: "toggle", id })}
          />
        ) : null}
        {lens === "table" ? <TreeTableView rows={rows} /> : null}
        {lens === "sunburst" ? <SunburstView nodes={state.nodes} /> : null}
        {lens === "treemap" ? <TreemapView nodes={state.nodes} /> : null}
      </div>
    </div>
  );
}

function onTreeKey(event: KeyboardEvent, commit: (action: TreeAction) => void) {
  const key = event.key;
  const interesting = ["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight", "Home", "End", "F2", "Delete", "*", "z", "Escape"].includes(key);
  if (!interesting) return;
  if (event.target instanceof HTMLInputElement) return;
  event.preventDefault();
  commit({ type: "key", key, alt: event.altKey, meta: event.metaKey || event.ctrlKey, shift: event.shiftKey });
}
