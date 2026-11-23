import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ClipboardListIcon, DatabaseIcon, FolderTreeIcon, MessagesSquareIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";

import { fetchSessionPage } from "@/lib/api/sessions.ts";
import { useInspectorStore } from "@/lib/store/inspector.ts";
import { useStatusStore } from "@/lib/store/status.ts";
import { shortId } from "@/lib/inspector/format.ts";
import { cn } from "@/lib/utils";

import { FileTree } from "@/components/files/file-tree.tsx";
import { DeleteSessionDialog } from "@/components/sessions/delete-session-dialog.tsx";
import { ActionsPanel } from "@/components/inspector/actions-panel.tsx";
import { InspectorRail } from "@/components/inspector/inspector-rail.tsx";
import { KnowledgePanel } from "@/components/inspector/knowledge-panel.tsx";
import { LogReader } from "@/components/inspector/log-reader.tsx";
import { PlanPanel } from "@/components/inspector/plan-panel.tsx";
import { UnavailableRow } from "@/components/inspector/section.tsx";
import { SessionList } from "@/components/sessions/session-list.tsx";

type PrimaryTab = "sessions" | "files" | "knowledge" | "plan";

const primaryTabs: { id: PrimaryTab; label: string; icon: typeof MessagesSquareIcon }[] = [
  { id: "sessions", label: "Sessions", icon: MessagesSquareIcon },
  { id: "files", label: "Files", icon: FolderTreeIcon },
  { id: "knowledge", label: "Knowledge", icon: DatabaseIcon },
  { id: "plan", label: "Plan", icon: ClipboardListIcon },
];

type Leaf =
  | { kind: "log"; sessionId: string; entryId: string }
  | { kind: "file"; path: string }
  | null;

function parseSessionPath(path: string): { sessionId: string; tail: string } | null {
  const match = /^\/sessions\/([^/]+)(?:\/(.*))?$/.exec(path);
  if (!match) return null;
  return { sessionId: match[1], tail: match[2] ?? "" };
}

export function InspectorPage() {
  const { sessionId: routeSessionId } = useParams<{ sessionId?: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState<PrimaryTab>("sessions");
  const [leaf, setLeaf] = useState<Leaf>(null);
  const [deleteSessionId, setDeleteSessionId] = useState<string | null>(null);
  const setSelectedTrace = useInspectorStore((state) => state.setSelectedTrace);
  const setTab = useInspectorStore((state) => state.setTab);
  const toggleInspector = useInspectorStore((state) => state.toggle);
  const setInspectorOpen = useInspectorStore((state) => state.setOpen);

  const firstSession = useQuery({
    queryKey: ["inspector", "first-session"],
    queryFn: () => fetchSessionPage({ limit: 1 }),
    enabled: !routeSessionId,
  });

  useEffect(() => {
    if (routeSessionId || !firstSession.data?.items.length) return;
    navigate(`/inspector/${encodeURIComponent(firstSession.data.items[0].sessionId)}`, { replace: true });
  }, [firstSession.data, navigate, routeSessionId]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || !event.shiftKey) return;
      if (event.key.toLowerCase() === "i") {
        event.preventDefault();
        toggleInspector();
        return;
      }
      if (["1", "2", "3", "4"].includes(event.key)) {
        event.preventDefault();
        const tab = (["status", "knowledge", "plan", "trace"] as const)[Number(event.key) - 1];
        setInspectorOpen(true);
        setTab(tab);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [setInspectorOpen, setTab, toggleInspector]);

  const selectSession = (sessionId: string) => {
    setLeaf(null);
    if (!sessionId) {
      navigate("/inspector");
      return;
    }
    useStatusStore.getState().ensure(sessionId);
    navigate(`/inspector/${encodeURIComponent(sessionId)}`);
    setSelectedTrace(null);
  };

  const openTrace = (path: string) => {
    const parsed = parseSessionPath(path);
    if (!parsed) return;
    const traceId = parsed.tail.replace(/^traces\//, "");
    navigate(`/inspector/${encodeURIComponent(parsed.sessionId)}`);
    setSelectedTrace(traceId || null);
    setTab("trace");
    setInspectorOpen(true);
  };

  const openLog = (path: string) => {
    const parsed = parseSessionPath(path);
    if (!parsed) return;
    navigate(`/inspector/${encodeURIComponent(parsed.sessionId)}`);
    setLeaf({ kind: "log", sessionId: parsed.sessionId, entryId: parsed.tail.replace(/^logs\//, "") });
  };

  const openFile = (path: string) => {
    const parsed = parseSessionPath(path);
    if (!parsed) return;
    navigate(`/inspector/${encodeURIComponent(parsed.sessionId)}`);
    setLeaf({ kind: "file", path });
  };

  return (
    <div className="bg-background flex h-dvh w-full overflow-hidden">
      <aside className="border-border/70 bg-sidebar hidden w-56 shrink-0 flex-col border-r md:flex" aria-label="Inspector navigation">
        <div className="flex items-center gap-2 px-3 py-3">
          <span className="accent-gradient flex size-7 items-center justify-center rounded-[8px] text-white shadow-elev-2">
            <span className="font-mono text-[11px] font-semibold">MM</span>
          </span>
          <div className="min-w-0">
            <p className="truncate text-[13px] font-semibold tracking-tight">Inspector</p>
            <p className="truncate font-mono text-[10px] text-muted-foreground">console · phase 11</p>
          </div>
        </div>
        <nav className="flex flex-col gap-0.5 px-2" aria-label="Inspector sections">
          {primaryTabs.map((entry) => {
            const Icon = entry.icon;
            const active = activeTab === entry.id;
            return (
              <button
                aria-current={active ? "page" : undefined}
                className={cn(
                  "flex items-center gap-2 rounded-sm px-2 py-1.5 text-left text-[12px]",
                  active ? "bg-surface text-foreground shadow-elev-1" : "text-muted-foreground hover:text-foreground",
                )}
                key={entry.id}
                onClick={() => {
                  setActiveTab(entry.id);
                  setLeaf(null);
                }}
                type="button"
              >
                <Icon className="size-3.5" />
                {entry.label}
              </button>
            );
          })}
        </nav>
        <div className="mt-auto flex flex-col gap-1 p-2">
          {routeSessionId ? (
            <p className="truncate font-mono text-[10px] text-muted-foreground" title={routeSessionId}>
              session {shortId(routeSessionId)}
            </p>
          ) : (
            <p className="text-[10px] text-muted-foreground">No session selected</p>
          )}
          <p className="text-[10px] text-muted-foreground">⌘⇧I inspector · ⌘⇧1–4 tabs</p>
        </div>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col">
        <header className="border-border/70 flex h-9 shrink-0 items-center gap-2 border-b px-2">
          <div className="flex items-center gap-0.5 md:hidden">
            {primaryTabs.map((entry) => (
              <button
                aria-pressed={activeTab === entry.id}
                className={cn(
                  "rounded-sm px-1.5 py-1 text-[10px]",
                  activeTab === entry.id ? "bg-surface text-foreground" : "text-muted-foreground",
                )}
                key={entry.id}
                onClick={() => {
                  setActiveTab(entry.id);
                  setLeaf(null);
                }}
                type="button"
              >
                {entry.label}
              </button>
            ))}
          </div>
          <h1 className="hidden text-[12px] font-medium capitalize md:block">{activeTab}</h1>
          <span className="min-w-0 flex-1 truncate text-center font-mono text-[10px] text-muted-foreground md:text-left">
            {routeSessionId ? routeSessionId : "No session selected"}
          </span>
          {routeSessionId ? (
            <button
              className="text-[10px] text-primary"
              onClick={() => {
                setSelectedTrace(null);
                setTab("trace");
                setInspectorOpen(true);
              }}
              type="button"
            >
              Trace
            </button>
          ) : null}
        </header>

        {!routeSessionId && activeTab !== "sessions" ? (
          <div className="p-4">
            <UnavailableRow section={activeTab} reason="select a session first" />
          </div>
        ) : null}

        {activeTab === "sessions" ? (
          <SessionList onSelect={selectSession} selectedId={routeSessionId ?? null} />
        ) : null}

        {activeTab === "files" && routeSessionId ? (
          leaf ? (
            <div className="flex min-h-0 flex-1 flex-col">
              <div className="border-border/70 flex items-center gap-2 border-b px-3 py-1.5">
                <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-muted-foreground">
                  {leaf.kind === "log" ? `logs/${leaf.entryId}` : leaf.path}
                </span>
                <button className="text-[10px] text-primary" onClick={() => setLeaf(null)} type="button">
                  Back to files
                </button>
              </div>
              {leaf.kind === "log" ? (
                <LogReader entryId={leaf.entryId} onClose={() => setLeaf(null)} sessionId={leaf.sessionId} />
              ) : (
                <div className="p-3">
                  <UnavailableRow section="file preview" reason="this console route exposes session files as tree nodes only" />
                  <p className="mt-2 font-mono text-[10px] text-muted-foreground">{leaf.path}</p>
                </div>
              )}
            </div>
          ) : (
            <FileTree
              onDeleteSession={setDeleteSessionId}
              onOpenFile={openFile}
              onOpenLog={openLog}
              onOpenTrace={openTrace}
            />
          )
        ) : null}

        {activeTab === "knowledge" && routeSessionId ? (
          <KnowledgePanel
            onOpenTrace={(traceId) => {
              setSelectedTrace(traceId);
              setTab("trace");
              setInspectorOpen(true);
            }}
          />
        ) : null}

        {activeTab === "plan" && routeSessionId ? (
          <div className="flex min-h-0 flex-1 overflow-hidden">
            <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden border-r border-border/70">
              <PlanPanel sessionId={routeSessionId} />
            </div>
            <div className="hidden min-h-0 w-[360px] flex-col overflow-hidden lg:flex">
              <ActionsPanel sessionId={routeSessionId} />
            </div>
          </div>
        ) : null}
      </main>

      <InspectorRail sessionId={routeSessionId ?? null} onNavigate={(target) => setActiveTab(target)} />
      <DeleteSessionDialog
        onDeleted={(sessionId) => {
          void queryClient.invalidateQueries({ queryKey: ["inspector", "sessions"] });
          if (sessionId === routeSessionId) selectSession("");
        }}
        onOpenChange={(open) => {
          if (!open) setDeleteSessionId(null);
        }}
        session={deleteSessionId ? { sessionId: deleteSessionId } : null}
      />
    </div>
  );
}
