import { useEffect, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { Maximize2, Minimize2, PanelLeftClose } from "lucide-react";

import { ChatPane } from "@/components/chat/chat-pane.tsx";
import { TreeCore } from "@/components/trees/tree-core.tsx";
import { DocShell } from "@/components/docs/doc-shell.tsx";
import { useHealth } from "@/lib/api/health.ts";
import { fetchDoc } from "@/lib/api/focus.ts";
import { activeDocument, useDocuments } from "@/lib/store/documents.ts";
import { hrefFor, useFocusedDocument, type DocumentMeta } from "@/lib/nav.ts";
import { useFocusStore } from "@/lib/store/focus.ts";
import { usePaneStore, type PaneId } from "@/lib/store/pane.ts";
import { TONE_EDGE } from "@/lib/tone.ts";
import { useViewportTier } from "@/lib/use-tier.ts";
import { WIRE_VERSION, type ServiceStatus } from "@/lib/wire/index.ts";
import { cn } from "@/lib/utils.ts";
import { WorkspaceSplit } from "@/components/shell/workspace-split.tsx";

const STARTERS = ["doctor", "logs", "budgets", "being"] as const;

export function Workspace() {
  const focused = useFocusedDocument();
  const onWorkspace = focused.id === "workspace";

  return (
    <div className="h-full min-h-0">
      <WorkspaceSplit
        document={
          <PaneChrome pane="document" title={onWorkspace ? "Document" : focused.title} tone={focused.tone}>
            <DocumentBody meta={focused} />
          </PaneChrome>
        }
        chat={
          <PaneChrome pane="chat" title="Chat" tone="violet">
            <ChatPane />
          </PaneChrome>
        }
      />
    </div>
  );
}

function PaneChrome({
  pane,
  title,
  tone,
  children,
}: {
  pane: PaneId;
  title: string;
  tone: DocumentMeta["tone"];
  children: ReactNode;
}) {
  const tier = useViewportTier();
  const active = usePaneStore((state) => state.activePane === pane);
  const fullscreen = usePaneStore((state) => state.fullscreen);
  const mode = useFocusStore((state) => state.mode);
  const toggleCollapsed = usePaneStore((state) => state.toggleCollapsed);
  const toggleFullscreen = usePaneStore((state) => state.toggleFullscreen);
  const setActivePane = usePaneStore((state) => state.setActivePane);
  const setMode = useFocusStore((state) => state.setMode);
  const showMode = pane === "document" && tier !== "monitor";

  return (
    <main
      aria-label={pane === "document" ? "Document pane" : "Chat pane"}
      data-pane={pane}
      tabIndex={-1}
      className={cn(
        "bg-background flex h-full min-h-0 flex-col border-l-[3px] outline-none",
        TONE_EDGE[tone],
        active && "ring-azure-500 ring-1 ring-inset",
      )}
      onFocus={() => setActivePane(pane)}
    >
      <header className="border-border flex h-8 shrink-0 items-center gap-2 border-b px-2">
        <h2 className="text-h3 min-w-0 flex-1 truncate">{title}</h2>
        {showMode ? (
          <div className="border-border flex overflow-hidden rounded-md border" role="group" aria-label="Document mode">
            <ModeButton current={mode} value="view" onSelect={setMode} />
            <ModeButton current={mode} value="edit" onSelect={setMode} />
          </div>
        ) : null}
        <IconAction
          label={fullscreen ? "Exit fullscreen" : `Fullscreen ${title}`}
          data-expand={pane}
          onClick={() => {
            setActivePane(pane);
            toggleFullscreen();
          }}
        >
          {fullscreen ? <Minimize2 className="size-3.5" /> : <Maximize2 className="size-3.5" />}
        </IconAction>
        {fullscreen ? null : (
          <IconAction label={`Collapse ${title}`} data-collapse={pane} onClick={() => toggleCollapsed(pane)}>
            <PanelLeftClose className="size-3.5" />
          </IconAction>
        )}
      </header>
      <div data-pane-body className="pane-scroll min-h-0 flex-1 overflow-auto">
        {children}
      </div>
    </main>
  );
}

function ModeButton({
  current,
  value,
  onSelect,
}: {
  current: "view" | "edit";
  value: "view" | "edit";
  onSelect: (mode: "view" | "edit") => void;
}) {
  const selected = current === value;
  return (
    <button
      type="button"
      aria-pressed={selected}
      className={cn(
        "text-micro h-6 px-2 uppercase",
        selected ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-muted",
      )}
      onClick={() => onSelect(value)}
    >
      {value}
    </button>
  );
}

function IconAction({
  label,
  onClick,
  children,
  ...rest
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className="text-muted-foreground hover:bg-muted hover:text-foreground grid size-6 place-items-center rounded-md"
      onClick={onClick}
      {...rest}
    >
      {children}
    </button>
  );
}

function DocumentBody({ meta }: { meta: DocumentMeta }) {
  const mode = useFocusStore((state) => state.mode);
  const tier = useViewportTier();
  const editing = mode === "edit" && tier !== "monitor";
  const active = useDocuments((state) => activeDocument(state));
  const open = useDocuments((state) => state.open);
  const focusId =
    meta.id === "logs" ? "01jdoclog00000000000000000" : meta.id === "runs" ? "01jdocrun00000000000000000" : null;
  const showFocus = Boolean(active) && (meta.kind === "empty" || meta.kind === "health" || Boolean(focusId));

  useEffect(() => {
    if (!focusId) return;
    let cancelled = false;
    void fetchDoc(focusId).then((doc) => {
      if (!cancelled) open(doc);
    }).catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [focusId, open]);

  if (showFocus) return <DocShell />;
  if (meta.kind === "empty") return <EmptyDocument />;
  if (meta.kind === "tree") return <TreeCore name={meta.id} />;

  if (editing) {
    return (
      <div className="flex flex-col gap-2 p-4">
        <p className="micro-label text-muted-foreground">Edit</p>
        <p className="text-body max-w-md">
          Editors write drafts. Open a document from the workspace to edit it.
        </p>
      </div>
    );
  }
  if (meta.kind === "health") return <HealthDocument meta={meta} />;
  if (meta.kind === "about") return <AboutDocument />;
  return (
    <div className="flex flex-col gap-2 p-4">
      <p className="micro-label text-amber-700 dark:text-amber-300">Unavailable</p>
      <p className="text-body max-w-md">{meta.unavailable}</p>
    </div>
  );
}

const DOC_OPEN = [
  ["01jdocprose000000000000000", "Note"],
  ["01jdocchart000000000000000", "Chart"],
  ["01jdoclog00000000000000000", "Log"],
  ["graph", "Graph"],
] as const;

function EmptyDocument() {
  const open = useDocuments((state) => state.open);
  return (
    <div className="flex h-full flex-col justify-center gap-4 p-6">
      <div>
        <p className="text-h2">Focus a document or ask the kernel</p>
        <p className="text-small text-muted-foreground mt-1 max-w-md">
          The workspace is empty. Open a document, or apply a focus frame from the chat pane.
        </p>
      </div>
      <div className="grid max-w-lg grid-cols-2 gap-2">
        {DOC_OPEN.map(([id, label]) => (
          <button
            key={id}
            type="button"
            className="border-border hover:bg-muted rounded-xs border p-3 text-left"
            onClick={() => void fetchDoc(id).then(open)}
          >
            <p className="text-h3">{label}</p>
            <p className="text-small text-muted-foreground mt-1">Open</p>
          </button>
        ))}
        {STARTERS.map((id) => (
          <Link key={id} to={hrefFor(id)} className="border-border hover:bg-muted rounded-xs border p-3">
            <p className="text-h3 capitalize">{id}</p>
            <p className="text-small text-muted-foreground mt-1">Open</p>
          </Link>
        ))}
      </div>
    </div>
  );
}

function HealthDocument({ meta }: { meta: DocumentMeta }) {
  const health = useHealth();
  return (
    <div className="flex flex-col gap-4 p-4">
      <p className="text-small text-muted-foreground max-w-xl">
        {meta.id === "doctor"
          ? "This is the live health envelope. The full doctor report arrives with the operate surfaces."
          : "Kernel health from GET /api/health. Services that are down say why."}
      </p>
      {health.isPending ? <p className="text-body">Checking the kernel…</p> : null}
      {health.isError ? (
        <p className="text-body text-red-700 dark:text-red-300">
          Unreachable. {health.error instanceof Error ? health.error.message : "The health request failed."}
        </p>
      ) : null}
      {health.data ? (
        <div className="flex flex-col gap-3">
          <p className="font-mono text-value">
            kernel {health.data.kernel.state}
            {health.data.kernel.reason ? ` — ${health.data.kernel.reason}` : ""}
          </p>
          <ul className="flex flex-col gap-2">
            {health.data.kernel.services.map((service) => (
              <ServiceRow key={service.name} service={service} />
            ))}
          </ul>
          <p className="text-small text-muted-foreground font-mono break-all">
            events {health.data.eventHead.seq} · {health.data.dataDir}
          </p>
        </div>
      ) : null}
    </div>
  );
}

function ServiceRow({ service }: { service: ServiceStatus }) {
  return (
    <li className="border-border rounded-xs flex items-baseline justify-between gap-3 border px-2 py-1">
      <span className="font-mono text-value">{service.name}</span>
      <span className="text-small text-muted-foreground text-right">
        {service.state}
        {service.reason ? ` — ${service.reason}` : ""}
      </span>
    </li>
  );
}

function AboutDocument() {
  const health = useHealth();
  return (
    <div className="flex flex-col gap-2 p-4">
      <p className="text-h2">Metamind Console</p>
      <p className="font-mono text-value">wire {WIRE_VERSION}</p>
      <p className="text-small text-muted-foreground font-mono break-all">
        {health.data ? health.data.dataDir : "Data directory arrives with the health check."}
      </p>
    </div>
  );
}

