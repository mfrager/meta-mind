export type SidebarTab = "sessions" | "projects" | "files";

/** Two-pane workspace (default), or either pane filling the workspace. */
export type PaneLayout = "two" | "chat" | "workspace";

export type ReasoningEffort = "low" | "medium" | "high";

export type TransportMode = "server" | "mock";

export interface Session {
  id: string;
  title: string;
  summary?: string;
  projectId?: string;
  updatedAt: string;
  messageCount: number;
  status: "active" | "idle" | "archived";
  pinned?: boolean;
  tags?: string[];
}

export interface ProjectResource {
  id: string;
  label: string;
  kind: "document" | "repository" | "dataset" | "artifact";
}

export interface Project {
  id: string;
  name: string;
  description: string;
  updatedAt: string;
  accent: "blue" | "teal" | "slate" | "green";
  resources: ProjectResource[];
  pinned?: boolean;
}

export interface FileEntry {
  id: string;
  name: string;
  path: string;
  kind: "file" | "folder";
  origin: "upload" | "artifact" | "document";
  language?: string;
  size?: number;
  updatedAt?: string;
  children?: FileEntry[];
}

export type ArtifactKind =
  | "document"
  | "code"
  | "json"
  | "table"
  | "chart"
  | "terminal"
  | "plan"
  | "files"
  | "preview";

export type ArtifactSyncStatus = "ready" | "generating" | "synced" | "failed";

export interface Artifact {
  id: string;
  title: string;
  kind: ArtifactKind;
  language?: string;
  content: string;
  dirty?: boolean;
  status: ArtifactSyncStatus;
  origin: "agent" | "user" | "file";
  sessionId?: string;
  updatedAt?: string;
}

export interface SourceRef {
  index: number;
  title: string;
  url: string;
  snippet?: string;
}

export type StepStatus = "pending" | "running" | "done" | "error";

export interface TraceStep {
  id: string;
  kind: "tool" | "search" | "plan" | "subagent";
  title: string;
  description?: string;
  status: StepStatus;
  startedAt?: number;
  durationMs?: number;
  input?: unknown;
  output?: string;
  steps?: string[];
}

export type PlanStepStatus = "pending" | "running" | "done";

export interface PlanStep {
  id: string;
  label: string;
  status: PlanStepStatus;
}

export interface PlanPayload {
  title: string;
  description?: string;
  steps: PlanStep[];
}

export interface QuestionnaireOption {
  id: string;
  label: string;
  description?: string;
}

export interface QuestionnairePayload {
  id: string;
  question: string;
  options: QuestionnaireOption[];
  multiSelect?: boolean;
}

export interface ConfirmationPayload {
  id: string;
  action: string;
  detail: string;
  toolName?: string;
  accepted?: boolean;
}

export type MessagePart =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string; done?: boolean }
  | { type: "trace"; steps: TraceStep[] }
  | { type: "plan"; plan: PlanPayload }
  | { type: "sources"; sources: SourceRef[] }
  | { type: "questionnaire"; payload: QuestionnairePayload }
  | { type: "confirmation"; payload: ConfirmationPayload }
  | { type: "subagent"; name: string; model: string; status: StepStatus; steps: TraceStep[] };

export type MessageStatus = "streaming" | "complete" | "error" | "cancelled";

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  createdAt: string;
  parts: MessagePart[];
  status: MessageStatus;
  model?: string;
  error?: string;
  usage?: { prompt?: number; completion?: number; total?: number };
}

export interface AgentUsage {
  prompt?: number;
  completion?: number;
  total?: number;
}

export type AgentEvent =
  | { type: "start"; messageId: string; model: string }
  | { type: "thinking"; delta: string }
  | { type: "text"; delta: string }
  | {
      type: "tool";
      call: {
        id: string;
        name: string;
        status: StepStatus;
        description?: string;
        input?: unknown;
        output?: string;
      };
    }
  | { type: "plan"; plan: PlanPayload }
  | { type: "sources"; sources: SourceRef[] }
  | { type: "questionnaire"; payload: QuestionnairePayload }
  | { type: "confirmation"; payload: ConfirmationPayload }
  | { type: "artifact"; artifact: Artifact }
  | {
      type: "subagent";
      name: string;
      model: string;
      status: StepStatus;
      steps?: TraceStep[];
    }
  | { type: "usage"; usage: AgentUsage }
  | { type: "done"; finishReason?: string; usage?: AgentUsage }
  | { type: "error"; message: string };

export interface ChatRequestPayload {
  sessionId: string;
  prompt: string;
  model: string;
  system?: string;
  reasoningEffort: ReasoningEffort;
  temperature?: number;
  attachments?: { name: string; size?: number }[];
}

export interface ServerHealth {
  status: "ok" | "degraded";
  backend: string;
  model: string;
  demo: boolean;
  version: string;
}
