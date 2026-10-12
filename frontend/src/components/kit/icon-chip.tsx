import {
  Activity,
  Box,
  Brain,
  FileText,
  Flag,
  Gauge,
  GitCompare,
  GitFork,
  ChartLine,
  Library,
  Network,
  Scale,
  ScrollText,
  ShieldCheck,
  Target,
  Wrench,
  type LucideIcon,
} from "lucide-react";

import { cn } from "@/lib/utils.ts";
import { TONE_CHIP, type Tone } from "@/lib/tone.ts";

export const ICON_KINDS = {
  doc: { icon: FileText, tone: "slate", label: "Document" },
  chart: { icon: ChartLine, tone: "cyan", label: "Chart" },
  log: { icon: ScrollText, tone: "indigo", label: "Log" },
  tree: { icon: Network, tone: "teal", label: "Tree" },
  mindmap: { icon: GitFork, tone: "rose", label: "Mind map" },
  diff: { icon: GitCompare, tone: "amber", label: "Diff" },
  proof: { icon: ShieldCheck, tone: "green", label: "Proof" },
  run: { icon: Activity, tone: "violet", label: "Run" },
  memory: { icon: Brain, tone: "teal", label: "Memory" },
  claim: { icon: Scale, tone: "amber", label: "Claim" },
  goal: { icon: Flag, tone: "rose", label: "Goal" },
  module: { icon: Box, tone: "indigo", label: "Module" },
  tool: { icon: Wrench, tone: "indigo", label: "Tool" },
  budget: { icon: Gauge, tone: "amber", label: "Budget" },
  policy: { icon: Scale, tone: "amber", label: "Policy" },
  frame: { icon: Library, tone: "teal", label: "Frame" },
  skill: { icon: Target, tone: "teal", label: "Skill" },
  case: { icon: Library, tone: "teal", label: "Case" },
  decision: { icon: Scale, tone: "azure", label: "Decision" },
  episode: { icon: Activity, tone: "violet", label: "Episode" },
} as const satisfies Record<string, { icon: LucideIcon; tone: Tone; label: string }>;

export type IconChipKind = keyof typeof ICON_KINDS;

const sizes = { 12: "size-4", 14: "size-5", 16: "size-6", 20: "size-7" } as const;

export function IconChip({
  kind,
  size = 14,
  tone,
  spin = false,
  dimmed = false,
  count,
}: {
  kind: IconChipKind;
  size?: 12 | 14 | 16 | 20;
  tone?: Tone;
  spin?: boolean;
  dimmed?: boolean;
  count?: number;
}) {
  const spec = ICON_KINDS[kind];
  const Icon = spec.icon;
  return (
    <span
      data-token={`icon-chip ${kind}`}
      className={cn("relative grid place-items-center rounded-xs", sizes[size], TONE_CHIP[tone ?? spec.tone], dimmed && "opacity-40")}
      aria-hidden={count === undefined}
    >
      <Icon className={cn("size-3.5", spin && "motion-safe:animate-spin")} />
      {count !== undefined ? (
        <span className="bg-red-500 text-primary-foreground absolute -top-1 -right-1 min-w-3 rounded-pill px-0.5 text-center text-[9px] leading-3">
          {count}
        </span>
      ) : null}
    </span>
  );
}

export function KindBadge({ kind }: { kind: IconChipKind }) {
  const spec = ICON_KINDS[kind];
  return (
    <span data-token={`kind-badge ${kind}`} className="inline-flex items-center gap-1">
      <IconChip kind={kind} size={12} />
      <span className="micro-label">{spec.label}</span>
    </span>
  );
}
