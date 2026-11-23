import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import type { Artifact, ArtifactKind, FileEntry } from "@/agentgui/types/workspace";

const languageToKind: Record<string, ArtifactKind> = {
  markdown: "document",
  mdx: "document",
  json: "json",
  csv: "table",
  ts: "code",
  typescript: "code",
  tsx: "code",
  js: "code",
  javascript: "code",
  rust: "code",
  python: "code",
  bash: "terminal",
  sh: "terminal",
};

export function kindForFile(file: FileEntry): ArtifactKind {
  if (file.language) return languageToKind[file.language] ?? "code";
  return "document";
}

const filePlaceholder = (file: FileEntry) => `# ${file.name}

_This ${file.language ?? "file"} is available in the connected workspace._

Path: \`${file.path}\`
`;

/** Find the artifact that represents a file, creating one on first open. */
export function artifactForFile(file: FileEntry): Artifact {
  const { artifacts } = useWorkspaceStore.getState();
  const existing = Object.values(artifacts).find(
    (artifact) => artifact.title === file.name && artifact.origin !== "agent"
  );
  if (existing) return existing;

  return {
    id: `file:${file.id}`,
    title: file.name,
    kind: kindForFile(file),
    language: file.language,
    content: filePlaceholder(file),
    origin: "file",
    status: "synced",
    updatedAt: file.updatedAt,
  };
}

export function openFileInWorkspace(
  file: FileEntry,
  options?: { activate?: boolean }
) {
  useWorkspaceStore.getState().openArtifact(artifactForFile(file), options);
  useWorkspaceStore.getState().selectFile(file.id);
}

export function openTraceInWorkspace(input: {
  sessionId: string;
  traceId: string;
  agentstreamId?: string;
}): void {
  const short = input.traceId.length > 12 ? `${input.traceId.slice(0, 8)}…` : input.traceId;
  const artifact: Artifact = {
    id: `trace:${input.sessionId}:${input.traceId}`,
    title: `log ${short}`,
    kind: "trace",
    content: "",
    origin: "server",
    status: "synced",
    sessionId: input.sessionId,
    traceId: input.traceId,
    agentstreamId: input.agentstreamId,
    updatedAt: new Date().toISOString(),
  };
  const store = useWorkspaceStore.getState();
  store.openArtifact(artifact, { activate: true });
  // `openArtifact` does not reveal a hidden pane; activating the tab does.
  store.setActiveArtifact(artifact.id);
}

export function openArtifactById(id: string) {
  const { artifacts, openArtifact } = useWorkspaceStore.getState();
  const artifact = artifacts[id];
  if (artifact) openArtifact(artifact, { activate: true });
}
