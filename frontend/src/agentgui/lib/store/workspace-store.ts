import { create } from "zustand";
import { persist } from "zustand/middleware";

import { bindSession } from "@/agentgui/services/agent-transport";
import { createId, titleFromPrompt } from "@/agentgui/lib/utils";
import type {
  Artifact,
  FileEntry,
  PaneLayout,
  Project,
  ReasoningEffort,
  Session,
  SidebarTab,
  TransportMode,
} from "@/agentgui/types/workspace";

interface WorkspaceState {
  /* navigation */
  sidebarTab: SidebarTab;
  sidebarCompact: boolean;
  mobileSidebarOpen: boolean;
  searchQuery: string;
  expandedGroups: Record<string, boolean>;

  /* selection */
  sessions: Session[];
  projects: Project[];
  files: FileEntry[];
  activeSessionId: string;
  activeProjectId: string | null;
  activeFileId: string | null;

  /* panes */
  paneLayout: PaneLayout;
  splitRatio: number;

  /* artifacts / workspace tabs */
  artifacts: Record<string, Artifact>;
  openArtifactIds: string[];
  activeArtifactId: string | null;

  /* agent settings */
  model: string;
  reasoningEffort: ReasoningEffort;
  transport: TransportMode;
  systemPrompt: string;

  setSidebarTab: (tab: SidebarTab) => void;
  toggleSidebarCompact: () => void;
  setMobileSidebarOpen: (open: boolean) => void;
  setSearchQuery: (query: string) => void;
  toggleGroup: (id: string) => void;

  selectSession: (id: string) => void;
  createSession: (title?: string, projectId?: string) => string;
  renameSession: (id: string, title: string) => void;
  togglePinSession: (id: string) => void;
  archiveSession: (id: string) => void;
  deleteSession: (id: string) => void;
  selectProject: (id: string | null) => void;
  selectFile: (id: string | null) => void;

  setPaneLayout: (layout: PaneLayout) => void;
  setSplitRatio: (ratio: number) => void;

  openArtifact: (artifact: Artifact, options?: { activate?: boolean }) => void;
  closeArtifact: (id: string) => void;
  closeOtherArtifacts: (id: string) => void;
  setActiveArtifact: (id: string | null) => void;
  updateArtifactContent: (id: string, content: string) => void;
  saveArtifact: (id: string) => void;
  renameArtifact: (id: string, title: string) => void;

  setModel: (model: string) => void;
  setReasoningEffort: (effort: ReasoningEffort) => void;
  setTransport: (transport: TransportMode) => void;
  setSystemPrompt: (prompt: string) => void;

  /** Merge the sessions metamind persists on the server into the sidebar. */
  hydrateSessions: () => Promise<void>;
}


export const useWorkspaceStore = create<WorkspaceState>()(
  persist(
    (set, get) => ({
      sidebarTab: "sessions",
      sidebarCompact: false,
      mobileSidebarOpen: false,
      searchQuery: "",
      expandedGroups: {
        recent: true,
        pinned: true,
        archived: false,
        "p-atlas": true,
        "p-agentgui": false,
        documents: true,
        datasets: false,
        generated: true,
      },

      sessions: [],
      projects: [],
      files: [],
      activeSessionId: "",
      activeProjectId: null,
      activeFileId: null,

      paneLayout: "two",
      splitRatio: 40,

      artifacts: {},
      openArtifactIds: [],
      activeArtifactId: null,

      model: "gpt-4o-mini",
      reasoningEffort: "medium",
      transport: "server",
      systemPrompt:
        "You are a precise engineering assistant working inside an agent workspace. Prefer short, structured answers with concrete evidence.",

      setSidebarTab: (sidebarTab) => set({ sidebarTab }),
      toggleSidebarCompact: () =>
        set((state) => ({ sidebarCompact: !state.sidebarCompact })),
      setMobileSidebarOpen: (mobileSidebarOpen) => set({ mobileSidebarOpen }),
      setSearchQuery: (searchQuery) => set({ searchQuery }),
      toggleGroup: (id) =>
        set((state) => ({
          expandedGroups: {
            ...state.expandedGroups,
            [id]: !state.expandedGroups[id],
          },
        })),

      selectSession: (id) =>
        set((state) => ({
          activeSessionId: id,
          mobileSidebarOpen: false,
          paneLayout: state.paneLayout === "workspace" ? "two" : state.paneLayout,
        })),
      createSession: (title = "New session", projectId) => {
        const id = createId("s");
        const session: Session = {
          id,
          title,
          projectId,
          updatedAt: new Date().toISOString(),
          messageCount: 0,
          status: "active",
        };
        set((state) => ({
          sessions: [session, ...state.sessions],
          activeSessionId: id,
          sidebarTab: "sessions",
          mobileSidebarOpen: false,
          paneLayout: state.paneLayout === "workspace" ? "two" : state.paneLayout,
        }));
        return id;
      },
      renameSession: (id, title) =>
        set((state) => ({
          sessions: state.sessions.map((session) =>
            session.id === id ? { ...session, title } : session
          ),
        })),
      togglePinSession: (id) =>
        set((state) => ({
          sessions: state.sessions.map((session) =>
            session.id === id ? { ...session, pinned: !session.pinned } : session
          ),
        })),
      archiveSession: (id) =>
        set((state) => ({
          sessions: state.sessions.map((session) =>
            session.id === id
              ? {
                  ...session,
                  status: session.status === "archived" ? "idle" : "archived",
                }
              : session
          ),
        })),
      deleteSession: (id) =>
        set((state) => {
          const sessions = state.sessions.filter((session) => session.id !== id);
          const nextActive =
            state.activeSessionId === id
              ? sessions[0]?.id ?? ""
              : state.activeSessionId;
          return { sessions, activeSessionId: nextActive };
        }),
      selectProject: (activeProjectId) =>
        set({ activeProjectId, mobileSidebarOpen: false }),
      selectFile: (activeFileId) => set({ activeFileId }),

      setPaneLayout: (paneLayout) => set({ paneLayout }),
      setSplitRatio: (splitRatio) =>
        set({ splitRatio: Math.min(75, Math.max(24, splitRatio)) }),

      openArtifact: (artifact, options) =>
        set((state) => ({
          artifacts: { ...state.artifacts, [artifact.id]: artifact },
          openArtifactIds: state.openArtifactIds.includes(artifact.id)
            ? state.openArtifactIds
            : [...state.openArtifactIds, artifact.id],
          activeArtifactId:
            options?.activate === false
              ? state.activeArtifactId
              : artifact.id,
        })),
      closeArtifact: (id) =>
        set((state) => {
          const openArtifactIds = state.openArtifactIds.filter(
            (artifactId) => artifactId !== id
          );
          const activeArtifactId =
            state.activeArtifactId === id
              ? openArtifactIds[openArtifactIds.length - 1] ?? null
              : state.activeArtifactId;
          return { openArtifactIds, activeArtifactId };
        }),
      closeOtherArtifacts: (id) =>
        set((state) => ({
          openArtifactIds: state.openArtifactIds.includes(id) ? [id] : [],
          activeArtifactId: id,
        })),
      setActiveArtifact: (activeArtifactId) =>
        set({
          activeArtifactId,
          paneLayout:
            activeArtifactId !== null && get().paneLayout === "chat"
              ? "two"
              : get().paneLayout,
        }),
      updateArtifactContent: (id, content) =>
        set((state) => {
          const artifact = state.artifacts[id];
          if (!artifact) return state;
          return {
            artifacts: {
              ...state.artifacts,
              [id]: { ...artifact, content, dirty: true },
            },
          };
        }),
      renameArtifact: (id, title) =>
        set((state) => {
          const artifact = state.artifacts[id];
          if (!artifact) return state;
          return {
            artifacts: { ...state.artifacts, [id]: { ...artifact, title } },
          };
        }),
      saveArtifact: (id) =>
        set((state) => {
          const artifact = state.artifacts[id];
          if (!artifact) return state;
          return {
            artifacts: {
              ...state.artifacts,
              [id]: {
                ...artifact,
                dirty: false,
                status: "synced",
                updatedAt: new Date().toISOString(),
              },
            },
          };
        }),

      setModel: (model) => set({ model }),
      setReasoningEffort: (reasoningEffort) => set({ reasoningEffort }),
      setTransport: (transport) => set({ transport }),
      setSystemPrompt: (systemPrompt) => set({ systemPrompt }),

      hydrateSessions: async () => {
        try {
          const response = await fetch("/api/chat/sessions");
          if (!response.ok) return;
          const body = (await response.json()) as {
            sessions?: { id: string; title: string; runs?: number }[];
          };
          const serverSessions: Session[] = (body.sessions ?? []).map((entry) => {
            bindSession(entry.id, entry.id);
            return {
              id: entry.id,
              title: entry.title || "Chat",
              updatedAt: new Date().toISOString(),
              messageCount: entry.runs ?? 0,
              status: "idle" as const,
            };
          });
          set((state) => {
            const known = new Set(serverSessions.map((session) => session.id));
            const local = state.sessions.filter((session) => !known.has(session.id));
            const sessions = [...local, ...serverSessions];
            const activeSessionId = sessions.some(
              (session) => session.id === state.activeSessionId
            )
              ? state.activeSessionId
              : sessions[0]?.id ?? "";
            return { sessions, activeSessionId };
          });
        } catch {
          // The backend may not be up yet; the shell still works offline.
        }
      },
    }),
    {
      name: "metamind-workspace-v1",
      partialize: (state) => ({
        sidebarTab: state.sidebarTab,
        sidebarCompact: state.sidebarCompact,
        expandedGroups: state.expandedGroups,
        activeSessionId: state.activeSessionId,
        activeProjectId: state.activeProjectId,
        activeFileId: state.activeFileId,
        paneLayout: state.paneLayout,
        splitRatio: state.splitRatio,
        artifacts: state.artifacts,
        openArtifactIds: state.openArtifactIds,
        activeArtifactId: state.activeArtifactId,
        model: state.model,
        reasoningEffort: state.reasoningEffort,
        transport: state.transport,
        sessions: state.sessions,
      }),
    }
  )
);

/** Session title used when a prompt opens a brand-new session. */
export function maybeRetitleSession(sessionId: string, prompt: string): void {
  const state = useWorkspaceStore.getState();
  const session = state.sessions.find((entry) => entry.id === sessionId);
  if (session && (session.title === "New session" || !session.title)) {
    state.renameSession(sessionId, titleFromPrompt(prompt));
  }
}

export function findFileByName(name: string): FileEntry | undefined {
  const { files } = useWorkspaceStore.getState();
  const walk = (entries: FileEntry[]): FileEntry | undefined => {
    for (const entry of entries) {
      if (entry.name === name) return entry;
      if (entry.children) {
        const found = walk(entry.children);
        if (found) return found;
      }
    }
    return undefined;
  };
  return walk(files);
}
