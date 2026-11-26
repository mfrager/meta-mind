import type { Tone } from "@/lib/tone.ts";
import type { DocKind } from "@/lib/wire/focus.ts";
import type { IconChipKind } from "@/components/kit/icon-chip.tsx";

export const DOC_KIND_META: Record<DocKind, { tone: Tone; chip: IconChipKind; label: string }> = {
  prose: { tone: "slate", chip: "doc", label: "Document" },
  chart: { tone: "cyan", chip: "chart", label: "Chart" },
  log: { tone: "indigo", chip: "log", label: "Log" },
  table: { tone: "slate", chip: "doc", label: "Table" },
  tree: { tone: "teal", chip: "tree", label: "Tree" },
  mindmap: { tone: "rose", chip: "mindmap", label: "Mind map" },
  diff: { tone: "amber", chip: "diff", label: "Diff" },
  proof: { tone: "green", chip: "proof", label: "Proof" },
  code: { tone: "indigo", chip: "doc", label: "Code" },
  form: { tone: "azure", chip: "doc", label: "Form" },
  timeline: { tone: "violet", chip: "run", label: "Timeline" },
  run: { tone: "violet", chip: "run", label: "Run" },
  decision: { tone: "amber", chip: "decision", label: "Decision" },
  budget: { tone: "amber", chip: "budget", label: "Budget" },
};
