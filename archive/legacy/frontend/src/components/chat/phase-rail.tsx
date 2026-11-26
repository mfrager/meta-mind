import type { Phase } from "@/lib/wire/chat.ts";

const PHASES: { id: Phase; label: string; tone: string }[] = [
  { id: "plan", label: "Plan", tone: "border-violet-500" },
  { id: "act", label: "Act", tone: "border-indigo-500" },
  { id: "verify", label: "Verify", tone: "border-green-500" },
  { id: "repair", label: "Repair", tone: "border-amber-500" },
  { id: "final", label: "Final", tone: "border-azure-500" },
];

export function PhaseRail({ active }: { active: Phase }) {
  const index = PHASES.findIndex((phase) => phase.id === active);
  return (
    <ol data-phase-rail className="flex gap-2 text-xs" aria-label="Turn phases">
      {PHASES.map((phase, phaseIndex) => (
        <li
          key={phase.id}
          data-phase={phase.id}
          data-active={phase.id === active ? "true" : "false"}
          className={`border-l-2 pl-1 ${phase.tone} ${phaseIndex <= index ? "text-foreground" : "text-muted-foreground"}`}
        >
          {phase.label}
        </li>
      ))}
    </ol>
  );
}
