import type {
  Artifact,
  ChatMessage,
  FileEntry,
  Project,
  Session,
} from "@/agentgui/types/workspace";

const now = Date.now();
const minutesAgo = (m: number) => new Date(now - m * 60_000).toISOString();

export const mockSessions: Session[] = [
  {
    id: "s-eval",
    title: "Retrieval eval harness",
    summary: "Audit recall, propose a cheaper reranking path",
    projectId: "p-atlas",
    updatedAt: minutesAgo(4),
    messageCount: 12,
    status: "active",
    pinned: true,
    tags: ["eval", "rag"],
  },
  {
    id: "s-ranker",
    title: "Cross-encoder sweep",
    summary: "Compare ms-marco MiniLM against bge-reranker",
    projectId: "p-atlas",
    updatedAt: minutesAgo(48),
    messageCount: 8,
    status: "idle",
    tags: ["benchmark"],
  },
  {
    id: "s-trace",
    title: "Streaming trace UI",
    summary: "Collapsible execution traces for tool calls",
    projectId: "p-agentgui",
    updatedAt: minutesAgo(190),
    messageCount: 21,
    status: "idle",
    pinned: true,
  },
  {
    id: "s-memo",
    title: "Onboarding memo draft",
    summary: "Two-page brief for the platform review",
    projectId: "p-ops",
    updatedAt: minutesAgo(1_500),
    messageCount: 6,
    status: "idle",
    tags: ["writing"],
  },
  {
    id: "s-jsonl",
    title: "Event log schema migration",
    summary: "Parent/child causal chain edge cases",
    projectId: "p-agentgui",
    updatedAt: minutesAgo(2_780),
    messageCount: 15,
    status: "idle",
  },
  {
    id: "s-budget",
    title: "Token budget planning",
    summary: "Cost envelope for Q1 inference",
    projectId: "p-ops",
    updatedAt: minutesAgo(4_320),
    messageCount: 4,
    status: "idle",
  },
  {
    id: "s-old",
    title: "Prompt regression notes",
    summary: "Archived after the v2 prompt roll-out",
    updatedAt: minutesAgo(14_000),
    messageCount: 9,
    status: "archived",
  },
];

export const mockProjects: Project[] = [
  {
    id: "p-atlas",
    name: "Atlas retrieval",
    description: "Search quality, rerankers and the offline eval loop.",
    updatedAt: minutesAgo(9),
    accent: "blue",
    pinned: true,
    resources: [
      { id: "r-1", label: "eval-results.json", kind: "dataset" },
      { id: "r-2", label: "rank-quality.md", kind: "document" },
      { id: "r-3", label: "atlas-search", kind: "repository" },
    ],
  },
  {
    id: "p-agentgui",
    name: "Agent workspace",
    description: "This interface: chat, artifacts and execution traces.",
    updatedAt: minutesAgo(120),
    accent: "teal",
    pinned: true,
    resources: [
      { id: "r-4", label: "design_plan1.md", kind: "document" },
      { id: "r-5", label: "trace-viewer.tsx", kind: "artifact" },
      { id: "r-6", label: "agentgui", kind: "repository" },
    ],
  },
  {
    id: "p-ops",
    name: "Platform ops",
    description: "Inference budget, incident notes and onboarding.",
    updatedAt: minutesAgo(1_400),
    accent: "slate",
    resources: [
      { id: "r-7", label: "budget-q1.csv", kind: "dataset" },
      { id: "r-8", label: "onboarding.md", kind: "document" },
    ],
  },
  {
    id: "p-brand",
    name: "Brand voice",
    description: "Tone guidelines and approved sample copy.",
    updatedAt: minutesAgo(6_000),
    accent: "green",
    resources: [{ id: "r-9", label: "voice-guide.md", kind: "document" }],
  },
];

export const mockFiles: FileEntry[] = [
  {
    id: "f-docs",
    name: "Documents",
    path: "documents",
    kind: "folder",
    origin: "document",
    children: [
      {
        id: "f-1",
        name: "rank-quality.md",
        path: "documents/rank-quality.md",
        kind: "file",
        origin: "document",
        language: "markdown",
        size: 8_420,
        updatedAt: minutesAgo(11),
      },
      {
        id: "f-2",
        name: "design_plan1.md",
        path: "documents/design_plan1.md",
        kind: "file",
        origin: "document",
        language: "markdown",
        size: 12_960,
        updatedAt: minutesAgo(300),
      },
      {
        id: "f-3",
        name: "onboarding.md",
        path: "documents/onboarding.md",
        kind: "file",
        origin: "document",
        language: "markdown",
        size: 5_120,
        updatedAt: minutesAgo(1_500),
      },
    ],
  },
  {
    id: "f-data",
    name: "Datasets",
    path: "datasets",
    kind: "folder",
    origin: "upload",
    children: [
      {
        id: "f-4",
        name: "eval-results.json",
        path: "datasets/eval-results.json",
        kind: "file",
        origin: "upload",
        language: "json",
        size: 24_310,
        updatedAt: minutesAgo(20),
      },
      {
        id: "f-5",
        name: "latency.csv",
        path: "datasets/latency.csv",
        kind: "file",
        origin: "upload",
        language: "csv",
        size: 3_980,
        updatedAt: minutesAgo(1_900),
      },
    ],
  },
  {
    id: "f-gen",
    name: "Generated",
    path: "generated",
    kind: "folder",
    origin: "artifact",
    children: [
      {
        id: "f-6",
        name: "ranker.ts",
        path: "generated/ranker.ts",
        kind: "file",
        origin: "artifact",
        language: "typescript",
        size: 2_140,
        updatedAt: minutesAgo(35),
      },
      {
        id: "f-7",
        name: "eval-report.md",
        path: "generated/eval-report.md",
        kind: "file",
        origin: "artifact",
        language: "markdown",
        size: 6_700,
        updatedAt: minutesAgo(6),
      },
    ],
  },
];

const rankerSource = `import { embedBatch, exactScan } from "./atlas/search";

export interface RankCandidate {
  id: string;
  score: number;
  text: string;
}

const RERANK_BUDGET = 24;

/**
 * Two-stage retrieval: a wide bi-encoder sweep, then a small cross-encoder
 * rerank over the head of the candidate list.
 */
export async function retrieve(
  query: string,
  topK = 8
): Promise<RankCandidate[]> {
  const [vector] = await embedBatch([query]);
  const candidates = await exactScan(vector, RERANK_BUDGET);

  const head = candidates.slice(0, RERANK_BUDGET);
  const tail = candidates.slice(RERANK_BUDGET);

  const rescored = await rerank(query, head);
  return [...rescored, ...tail].slice(0, topK);
}

async function rerank(
  query: string,
  candidates: RankCandidate[]
): Promise<RankCandidate[]> {
  // Batched cross-encoder call, cached per (query, doc) pair.
  const response = await fetch("/rerank", {
    method: "POST",
    body: JSON.stringify({ query, docs: candidates.map((c) => c.text) }),
  });
  const { scores } = (await response.json()) as { scores: number[] };
  return candidates
    .map((candidate, index) => ({ ...candidate, score: scores[index] }))
    .sort((a, b) => b.score - a.score);
}
`;

export const mockArtifacts: Record<string, Artifact> = {
  "a-report": {
    id: "a-report",
    title: "eval-report.md",
    kind: "document",
    language: "markdown",
    origin: "agent",
    status: "synced",
    sessionId: "s-eval",
    updatedAt: minutesAgo(6),
    content: `# Retrieval evaluation — 14 Nov

## Summary
Recall@10 holds at **0.91** after swapping the 350M reranker for the
24M cross-encoder. Median rerank latency drops from 186 ms to 74 ms.

| Stage | p50 | p95 | Recall@10 |
| --- | --- | --- | --- |
| bi-encoder only | 18 ms | 41 ms | 0.84 |
| + MiniLM rerank | 74 ms | 118 ms | 0.91 |
| + 350M rerank | 186 ms | 302 ms | 0.92 |

## Recommendation
Ship the MiniLM reranker with a candidate budget of 24. The larger model buys
0.01 recall for 2.5× the latency, which is not a trade worth making for
interactive search.

## Follow-ups
- Cache rerank scores on the (query, doc hash) pair.
- Add a golden set for the long-tail entity queries.
- Re-run the sweep once the new chunker lands.
`,
  },
  "a-ranker": {
    id: "a-ranker",
    title: "ranker.ts",
    kind: "code",
    language: "typescript",
    origin: "agent",
    status: "ready",
    sessionId: "s-eval",
    updatedAt: minutesAgo(35),
    content: rankerSource,
  },
  "a-json": {
    id: "a-json",
    title: "eval-results.json",
    kind: "json",
    language: "json",
    origin: "file",
    status: "synced",
    updatedAt: minutesAgo(20),
    content: JSON.stringify(
      {
        run_id: "eval_2026_11_14",
        dataset: "atlas-golden-v3",
        queries: 1840,
        models: [
          { name: "minilm-l6", recall_at_10: 0.91, p50_ms: 74, p95_ms: 118 },
          { name: "bge-reranker-base", recall_at_10: 0.92, p50_ms: 186, p95_ms: 302 },
        ],
        slices: { long_tail: 0.78, entity: 0.94, navigational: 0.96 },
      },
      null,
      2
    ),
  },
  "a-table": {
    id: "a-table",
    title: "latency.csv",
    kind: "table",
    language: "csv",
    origin: "file",
    status: "synced",
    updatedAt: minutesAgo(40),
    content: `stage,model,p50_ms,p95_ms,recall_at_10
retrieve,bi-encoder,18,41,0.84
rerank,minilm-l6,56,77,0.91
rerank,bge-base,168,261,0.92
generate,gpt-4o-mini,410,880,-
`,
  },
  "a-chart": {
    id: "a-chart",
    title: "recall-vs-latency",
    kind: "chart",
    origin: "agent",
    status: "ready",
    updatedAt: minutesAgo(52),
    content: JSON.stringify({
      unit: "ms",
      series: [
        { label: "bi-encoder", value: 18, meta: "0.84 recall" },
        { label: "MiniLM", value: 74, meta: "0.91 recall" },
        { label: "bge-base", value: 186, meta: "0.92 recall" },
        { label: "350M", value: 302, meta: "0.92 recall" },
      ],
    }),
  },
  "a-terminal": {
    id: "a-terminal",
    title: "eval run — terminal",
    kind: "terminal",
    origin: "agent",
    status: "ready",
    updatedAt: minutesAgo(58),
    content: `$ bun run eval --dataset atlas-golden-v3 --models minilm-l6,bge-base
▸ loading dataset           1840 queries
▸ building index            12,408 chunks
▸ bi-encoder sweep          recall@10 0.84  p50 18ms
▸ rerank minilm-l6          recall@10 0.91  p50 74ms
▸ rerank bge-base           recall@10 0.92  p50 186ms
✓ report written            generated/eval-report.md
`,
  },
  "a-plan": {
    id: "a-plan",
    title: "Migration plan",
    kind: "plan",
    origin: "agent",
    status: "ready",
    updatedAt: minutesAgo(75),
    content: JSON.stringify({
      title: "Reranker cut-over",
      description: "Move interactive search to the 24M cross-encoder.",
      steps: [
        { id: "p1", label: "Freeze the golden query set", status: "done" },
        { id: "p2", label: "Run the latency sweep", status: "done" },
        { id: "p3", label: "Cache rerank scores by doc hash", status: "running" },
        { id: "p4", label: "Roll out behind a 10% flag", status: "pending" },
      ],
    }),
  },
  "a-files": {
    id: "a-files",
    title: "generated/",
    kind: "files",
    origin: "agent",
    status: "ready",
    updatedAt: minutesAgo(90),
    content: JSON.stringify(mockFiles),
  },
  "a-design": {
    id: "a-design",
    title: "design_plan1.md",
    kind: "document",
    language: "markdown",
    origin: "file",
    status: "synced",
    updatedAt: minutesAgo(300),
    content: `# LLM agent workspace — design plan

## Layout
Two panes, independently scrollable. Chat on the left at roughly 40%, the
contextual workspace on the right at 60%, divided by a draggable divider.

## Chat
Streaming answers, collapsible execution traces, inline citations and a
composer that stays anchored to the bottom of the pane.

## Workspace
A tab manager for artifacts: documents, code, tables, charts, terminal output
and plans. Every view owns its own state and can be closed without disturbing
the conversation.

> Keep the shell independent from the agent interface so the workspace can
> evolve without reworking navigation.
`,
  },
};

export const mockConversation: ChatMessage[] = [
  {
    id: "m-seed-user",
    role: "user",
    createdAt: minutesAgo(9),
    status: "complete",
    parts: [
      {
        type: "text",
        text: "Audit the retrieval pipeline from the last eval run and tell me whether we can drop the 350M reranker. Open the report when you're done.",
      },
    ],
  },
  {
    id: "m-seed-assistant",
    role: "assistant",
    createdAt: minutesAgo(8),
    status: "complete",
    model: "gpt-4o-mini",
    usage: { prompt: 1_284, completion: 612, total: 1_896 },
    parts: [
      {
        type: "thinking",
        text: "The question is a cost/quality trade. I should pull the latest eval run, compare recall at each rerank stage, then check p95 latency against the 150 ms interaction budget.",
        done: true,
      },
      {
        type: "trace",
        steps: [
          {
            id: "t1",
            kind: "tool",
            title: "read_file",
            description: "datasets/eval-results.json",
            status: "done",
            durationMs: 180,
            input: { path: "datasets/eval-results.json" },
            output: "1840 queries · 2 models · recall slices for long-tail, entity, navigational",
          },
          {
            id: "t2",
            kind: "search",
            title: "grep",
            description: "rerank budget in atlas/search",
            status: "done",
            durationMs: 64,
            input: { pattern: "RERANK_BUDGET", path: "atlas/search" },
            output: "RERANK_BUDGET = 24",
          },
          {
            id: "t3",
            kind: "tool",
            title: "write_file",
            description: "generated/eval-report.md",
            status: "done",
            durationMs: 42,
            input: { path: "generated/eval-report.md" },
            output: "Report written (1.2 KB)",
          },
        ],
      },
      {
        type: "text",
        text: `Yes — the 350M reranker is no longer buying enough to justify its latency.

**What the run shows**

| Stage | p50 | p95 | Recall@10 |
| --- | --- | --- | --- |
| Bi-encoder only | 18 ms | 41 ms | 0.84 |
| + MiniLM rerank | 74 ms | 118 ms | 0.91 |
| + 350M rerank | 186 ms | 302 ms | 0.92 |

The large model adds **+0.01 recall** for **+112 ms p50** and a p95 that breaks
the 150 ms interaction budget. The 24M cross-encoder keeps recall at 0.91 and
sits comfortably inside it.

**Recommended cut-over**

1. Keep \`RERANK_BUDGET = 24\` — the recall curve flattens past 24 candidates.
2. Cache rerank scores per \`(query, doc_hash)\`; roughly 38% of queries repeat.
3. Roll out behind a 10% flag and watch the long-tail slice, currently 0.78.

I wrote the full comparison to \`generated/eval-report.md\`.`,
      },
      {
        type: "plan",
        plan: {
          title: "Reranker cut-over",
          description: "Move interactive search to the 24M cross-encoder.",
          steps: [
            { id: "p1", label: "Freeze the golden query set", status: "done" },
            { id: "p2", label: "Run the latency sweep", status: "done" },
            { id: "p3", label: "Cache rerank scores by doc hash", status: "running" },
            { id: "p4", label: "Roll out behind a 10% flag", status: "pending" },
          ],
        },
      },
      {
        type: "sources",
        sources: [
          {
            index: 1,
            title: "eval-results.json — run eval_2026_11_14",
            url: "atlas://datasets/eval-results.json",
            snippet: "recall_at_10: 0.91, p50_ms: 74 for minilm-l6",
          },
          {
            index: 2,
            title: "atlas/search/ranker.ts",
            url: "atlas://atlas/search/ranker.ts",
            snippet: "const RERANK_BUDGET = 24;",
          },
        ],
      },
    ],
  },
];
