// Wire types for the Phase 10 console routes.
//
// The status payload is camel-cased by the server, while a few adjacent
// endpoints (agent runs and the data layer) intentionally expose the Rust
// snake-case names. Keep those sides explicit so a panel never has to guess.

export type UnknownRecord = Record<string, unknown>;

export type StatusValue =
  | string
  | number
  | boolean
  | null
  | StatusValue[]
  | { [key: string]: StatusValue };

export type StateStatusView = {
  revision: number;
  topic: string | null;
  depth: number;
  status: Record<string, StatusValue>;
  updatedAt: number;
  updatedBy: string;
};

export type WorkStatusView = {
  id: string;
  title: string;
  kind: string;
  status: string;
  startedAt: number;
  lastActivityAt: number;
};

export type SessionStatusView = {
  sessionId: string;
  userId: string;
  lifecycle: string;
  presence: string;
  working: boolean;
  lastActivityAt: number;
  lastUserInputAt?: number | null;
  lastWorkActivityAt?: number | null;
  expiresAt?: number | null;
  traces: string[];
  traceCount: number;
  mainTraceId?: string | null;
  sessionDir?: string | null;
  resumeEpoch: number;
  deletedAt?: number | null;
  work: WorkStatusView[];
};

export type FrameStatusView = {
  id: string;
  templateId: string;
  templateVersion: number;
  kind: string;
  status: string;
  enteredAt: number;
  updatedAt: number;
  expiresAt?: number | null;
};

export type FramesStatusView = {
  revision: number;
  frames: FrameStatusView[];
  topFrameId?: string | null;
  pendingUtterance?: StatusValue;
};

export type PromptFragmentView = { id: string; version: number; chars: number };

export type PromptStatusView = {
  id: string;
  strategy: string;
  variant?: string | null;
  hash: string;
  fragments: PromptFragmentView[];
};

export type BudgetStatusView = {
  class: string;
  tokensUsed: number;
  tokenCap: number;
  turnsUsed: number;
  turnCap: number;
  estimated: boolean;
};

export type DataStatusView = {
  dataRevision: number;
  indexPending: number;
  indexLagMs: number;
  schemas: number;
  namespaces: number;
};

export type PlanStepStatusView = {
  id: string;
  title: string;
  status: string;
  attempts: number;
  runs: number;
};

export type PlanStatusView = {
  planId: string;
  goal: string;
  status: string;
  revision: number;
  currentStep: string | null;
  steps: PlanStepStatusView[];
};

export type PlansStatusView = { items: PlanStatusView[] };

export type ActionStatusView = {
  actionId: string;
  kind: string;
  status: string;
  dueAt: number | null;
  lastRunAt: number | null;
  runs: number;
};

export type ActionsStatusView = {
  scheduled: ActionStatusView[];
  recent: ActionStatusView[];
  nextRunAt: number | null;
};

export type AutomationStatusView = {
  supervisor: string;
  tickSecs: number;
  frameTimers: number;
  nextActionAt: number | null;
  sessions: number;
  cronJobs: number;
  lastReason: string | null;
};

export type InterjectionsStatusView = {
  policyId: string;
  suppressed: boolean;
  reason: string | null;
  queue: number;
};

export type AgentStatusView = {
  runId: string;
  kind: string;
  status: string;
  sessionId: string;
  updatedAt: number;
  active: boolean;
};

export type OperationStatusView = {
  id: string;
  kind: string;
  source: string;
  at: number;
  detail: StatusValue;
};

export type StatusView = {
  revision: number;
  sessionId: string;
  generatedAt: number;
  stateRevision: number;
  dataRevision: number;
  state: StateStatusView;
  session: SessionStatusView;
  frames: FramesStatusView;
  prompt: PromptStatusView | null;
  budget: BudgetStatusView;
  data: DataStatusView;
  plans: PlansStatusView;
  actions: ActionsStatusView;
  automation: AutomationStatusView;
  interjections: InterjectionsStatusView;
  agents: AgentStatusView[];
  operations: OperationStatusView[];
  unavailable: Record<string, string>;
  updatedAt: number;
};

export type StatusFrame = StatusView & { reason?: string };

export type SessionSummary = {
  sessionId: string;
  userId: string;
  topic: string | null;
  lifecycle: "active" | "ending" | "ended";
  presence: string;
  working: boolean;
  traceCount: number;
  traceKinds: string[];
  lastActivityAt: number;
  sessionDir: string | null;
  resumable: boolean;
  deleted: boolean;
};

export type SessionPage = {
  items: SessionSummary[];
  nextCursor: string | null;
  hasMore: boolean;
};

export type TraceKind =
  | "main"
  | "agent"
  | "plan"
  | "think"
  | "memory"
  | "action"
  | "automation"
  | "introspection"
  | "log";

export type SessionTrace = {
  traceId: string;
  sessionId: string;
  kind: TraceKind;
  agentstreamId: string;
  firstSeq: number;
  lastSeq: number;
  eventCount: number;
  startedAt: number;
  endedAt: number | null;
  sizeBytes: number;
};

export type TracePage = {
  items: SessionTrace[];
  nextCursor: string | null;
  hasMore: boolean;
};

export type ConversationStateView = {
  sessionId: string;
  revision: number;
  topic: string | null;
  depth: number;
  status: Record<string, StatusValue>;
  updatedAt: number;
  updatedBy: string;
};

export type FileTreeEntry = {
  name: string;
  kind: "dir" | "trace" | "log" | "file";
  path: string;
  size: number | null;
  at?: number;
  eventCount?: number;
  agentstreamId?: string;
};

export type FileTreePage = {
  path: string;
  entries: FileTreeEntry[];
  nextCursor: string | null;
  hasMore: boolean;
};

export type LogEntry = {
  id: string;
  source: string;
  level: string;
  kind: string;
  at: number;
  message: string;
  sequence?: number;
  traceId?: string | null;
  tags?: string[];
  refs?: string[];
};

export type LogPage = {
  entries: LogEntry[];
  nextCursor: string | null;
  hasMore: boolean;
  source?: string;
};

export type DataValue = {
  kind?: string;
  value?: unknown;
  iri?: string;
  datatype?: string | null;
  language?: string | null;
};

export type DataEntity = {
  iri: string;
  types: string[];
  label: string | null;
  properties: Record<string, DataValue[]>;
  graph: string;
  revision: number;
  status: string;
  created_at: number;
  updated_at: number;
  recorded_by: string;
};

export type DataSearchHit = {
  iri: string;
  type: string;
  label: string | null;
  score_micros: number;
  snippet: string | null;
  graph: string;
  revision: number;
  updated_at: number;
};

export type DataStatus = {
  entities: number;
  search_documents: number;
  index_pending: number;
  index_ready: boolean;
  data_revision: number;
  active_schemas: string[];
  last_mutation: number | null;
};

export type DataHealth = {
  status: DataStatus;
  health: {
    quads: number;
    graphs: number;
    ontology_triples: number;
    sqlite_schema: string;
  };
  schemas: number;
  namespaces: number;
};

export type SchemaSummary = {
  id: string;
  prefix: string;
  namespace: string;
  version: string;
  source: string;
  active: boolean;
  class_count: number;
  property_count: number;
  loaded_at: number | null;
};

export type NamespaceBinding = {
  prefix: string;
  namespace: string;
  builtin: boolean;
};

export type EntityPage = {
  items: Array<DataEntity | DataSearchHit>;
  hasMore: boolean;
  nextOffset: number | null;
  index?: unknown;
};

export type PlanView = {
  id: string;
  sessionId: string;
  goal: string;
  status: string;
  steps: Array<{
    id: string;
    title: string;
    detail: string | null;
    status: string;
    dependsOn: string[];
    action: string | null;
    result: string | null;
    attempts: number;
    runs: number;
    owner: string;
    agentRunId: string | null;
    verification: string | null;
    lastError: string | null;
  }>;
  currentStep: string | null;
  revision: number;
  createdAt: number;
  updatedAt: number;
  format: string;
  body: string;
  currentVersion: number;
  owner: string;
  monitor: boolean;
  description?: string | null;
  structure?: unknown;
  processUri?: string | null;
  expiresAt?: number | null;
  neverExpires?: boolean;
  docPath?: string | null;
  derivedTemplateUri?: string | null;
};

export type PlanVersion = {
  planId: string;
  version: number;
  format: string;
  body: string;
  steps: PlanView["steps"];
  createdAt: number;
  createdBy: string;
};

export type PlanList = {
  revision: number;
  items: PlanView[];
  monitor: boolean;
};

export type PlanDetail = {
  plan: PlanView;
  versions: PlanVersion[];
  monitor: boolean;
};

export type ProcessStepView = {
  status: string;
  attempts: number;
  output?: string | null;
  error?: string | null;
  startedAt?: number | null;
  finishedAt?: number | null;
  wait?: { kind: string; detail: string; wakeAt?: number | null } | null;
};

export type ProcessEventView = {
  seq: number;
  at: number;
  kind: string;
  stepId?: string | null;
  detail: string;
};

export type ProcessView = {
  id: string;
  templateUri: string;
  templateVersion: number;
  sessionId: string;
  status: string;
  steps: Record<string, ProcessStepView>;
  currentGroup?: string | null;
  waiting?: { kind: string; detail: string; wakeAt?: number | null } | null;
  parent?: string | null;
  depth: number;
  createdAt: number;
  updatedAt: number;
  revision: number;
  result?: string | null;
  error?: string | null;
  tokensIn: number;
  tokensOut: number;
  events: ProcessEventView[];
};

export type ProcessTemplateView = {
  uri: string;
  name: string;
  description: string;
  version: number;
  source: string;
  sessionId?: string | null;
  steps: Array<{
    id: string;
    title: string;
    instructions: string;
    dependsOn: string[];
    parallelGroup?: string | null;
    toolScope: string[];
    kind: string;
    check?: string | null;
    wait?: { kind: string; detail: string; wakeAt?: number | null } | null;
  }>;
  docPath?: string | null;
  status: string;
  deleted: boolean;
};

export type ProcessList = {
  enabled: boolean;
  processes: ProcessView[];
  templates: ProcessTemplateView[];
  analytics?: {
    total: number;
    live: number;
    completed: number;
    failed: number;
    cancelled: number;
    byTemplate: Record<string, { total: number; completed: number; failed: number; waiting: number; averageDurationMs: number }>;
  };
};

export type ScheduledActionRow = {
  actionId: string;
  kind: string;
  status: string;
  dueAt: number | null;
  lastRunAt: number | null;
  runs: number;
  schedule?: unknown;
};

export type ActionList = {
  sessionId: string | null;
  sessions: string[];
  scheduled: ScheduledActionRow[];
  recent: ScheduledActionRow[];
  nextRunAt: number | null;
};

export type AutomationInfo = {
  enabled: boolean;
  supervisor: string;
  sessions: string[];
  frameTimers: number;
  nextActionAt: number | null;
  config: {
    enabled: boolean;
    tickSecs: number;
    maxSessions: number;
    actionsEnabled: boolean;
    framesEnabled: boolean;
  } | null;
  planMonitor: boolean;
  internalActions: { disabled: string; maxCronJobs: string };
};

export type AgentRunView = {
  run_id: string;
  session_id: string;
  agentstream_id: string;
  kind: string;
  status: string;
  turns: number;
  task: string;
  origin: string;
  started_at: number;
  updated_at: number;
  last_summary: string | null;
  last_error: string | null;
  usage: { prompt_tokens: number; completion_tokens: number };
  workspace: string;
};

export type AgentstreamEvent = {
  event_id: string;
  session_id: string;
  turn_id: string | null;
  timestamp: number;
  event_type: string;
  payload: StatusValue;
  provenance: { source: string; model?: string | null };
};

export type TraceEventPage = {
  sessionId?: string;
  runId?: string;
  items: AgentstreamEvent[];
  next: number;
};

export type AgentList = { items: AgentRunView[]; active: number };

export type StateBoardPayload = {
  kernel: {
    state: string;
    reason?: string | null;
    services: { name: string; state: string; reason?: string | null }[];
  };
  being:
    | {
        ok: true;
        self_description: string;
        version: string;
        invariants: string[];
        goal_count: number;
        voice: {
          warmth: number;
          directness: number;
          curiosity: number;
          playfulness: number;
          formality: number;
        };
      }
    | { ok: false; reason: string };
  goals: { goals?: { id: string; description: string; priority: number; status: string }[] };
  budgets: {
    rows?: { kind: string; period: string; limit: number; spent: number }[];
    reason?: string;
  };
  last_action: { name: string; ok: boolean; summary: string } | null;
};
