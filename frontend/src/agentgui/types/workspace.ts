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
  | "trace"
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
  origin: "agent" | "user" | "file" | "server";
  sessionId?: string;
  /** Trace artifact: which agentstream log this tab shows. */
  traceId?: string;
  agentstreamId?: string;
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

export type PassName = "interpret" | "process" | "verify" | "think";

/** One model round inside a pass. Its text is phase prose, never the answer. */
export interface PassRound {
  round: number;
  model?: string;
  streaming: boolean;
  text: string;
  outcome?: string;
  error?: string;
  tokensIn?: number;
  tokensOut?: number;
  latencyMs?: number;
}

/** One verification check rendered inside the Verify pass. */
export interface PassCheck {
  label: string;
  pass: boolean;
  confidence: number;
  backend?: string;
  reasoning?: string;
  durationMs?: number;
}

/**
 * A pass card on the assistant message.
 *
 * It is display-only: the reply lives in the `text` part that follows, and phase
 * prose is collapsed by default so it can never be mistaken for the answer.
 */
export interface PassPart {
  type: "pass";
  pass: PassName;
  attempt: number;
  status: "running" | "done" | "failed";
  finished?: string;
  model?: string;
  rounds: PassRound[];
  tools: TraceStep[];
  checks: PassCheck[];
  retry?: { nextAttempt: number; reason: string };
  budget?: unknown;
}

export type MessagePart =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string; done?: boolean }
  | { type: "trace"; steps: TraceStep[] }
  | PassPart
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
        pass?: PassName;
        attempt?: number;
        round?: number;
        action?: string;
      };
    }
  | {
      type: "pass_start";
      pass: PassName;
      attempt: number;
      model?: string;
    }
  | {
      type: "pass_round";
      pass: PassName;
      attempt: number;
      round: number;
      model?: string;
      streaming: boolean;
    }
  | {
      type: "pass_delta";
      pass: PassName;
      attempt: number;
      round: number;
      delta: string;
    }
  | {
      type: "pass_round_end";
      pass: PassName;
      attempt: number;
      round: number;
      outcome: string;
      tokensIn?: number;
      tokensOut?: number;
      latencyMs?: number;
      error?: string;
    }
  | {
      type: "pass_complete";
      pass: PassName;
      attempt: number;
      finished: string;
      budget?: unknown;
    }
  | {
      type: "pass_retry";
      pass: PassName;
      attempt: number;
      nextAttempt: number;
      reason: string;
    }
  | {
      type: "verification";
      label: string;
      pass: boolean;
      confidence: number;
      attempt?: number;
      backend?: string;
      reasoning?: string;
      durationMs?: number;
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
