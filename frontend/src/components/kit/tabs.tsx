import { cn } from "@/lib/utils.ts";
import { IconChip, type IconChipKind } from "@/components/kit/icon-chip.tsx";

export function UnderlineTabs({ value }: { value: string }) {
  const tabs = ["Summary", "Data", "Raw"];
  return (
    <div role="tablist" aria-label="Document sections" data-token="tabs underline" className="border-border flex gap-3 border-b">
      {tabs.map((tab) => (
        <button key={tab} type="button" role="tab" aria-selected={tab === value} className={cn("text-small -mb-px border-b-2 pb-1", tab === value ? "border-azure-500" : "border-transparent")}>
          {tab}
        </button>
      ))}
    </div>
  );
}

export function SegmentedTabs({ value }: { value: string }) {
  const tabs = ["View", "Edit", "Diff", "Raw"];
  return (
    <div role="tablist" aria-label="Mode" data-token="tabs segmented" className="border-border elevation-1 inline-flex overflow-hidden rounded-md border">
      {tabs.map((tab) => (
        <button key={tab} type="button" role="tab" aria-selected={tab === value} className={cn("h-7 px-2 text-small", tab === value ? "bg-card" : "bg-muted")}>
          {tab}
        </button>
      ))}
    </div>
  );
}

export function PillTabs({ value }: { value: string }) {
  const tabs = [
    ["All", "12"],
    ["Failed", "2"],
    ["Promoted", "4"],
  ];
  return (
    <div role="tablist" aria-label="Filters" data-token="tabs pill" className="flex gap-1">
      {tabs.map(([tab, count]) => (
        <button key={tab} type="button" role="tab" aria-selected={tab === value} className={cn("micro-label rounded-pill border px-2 py-0.5", tab === value ? "border-azure-300 bg-azure-100" : "border-border")}>
          {tab} {count}
        </button>
      ))}
    </div>
  );
}

export function VerticalTabs({ value }: { value: string }) {
  const tabs: { id: string; kind: IconChipKind }[] = [
    { id: "Outline", kind: "tree" },
    { id: "Details", kind: "doc" },
  ];
  return (
    <div role="tablist" aria-label="Side" data-token="tabs vertical" className="flex w-36 flex-col">
      {tabs.map((tab) => (
        <button key={tab.id} type="button" role="tab" aria-selected={tab.id === value} className={cn("flex h-8 items-center gap-2 border-l-2 px-2 text-small", tab.id === value ? "border-azure-500 bg-accent" : "border-transparent")}>
          <IconChip kind={tab.kind} size={12} />
          {tab.id}
        </button>
      ))}
    </div>
  );
}

export function PaneTabs() {
  return (
    <div role="tablist" aria-label="Open documents" data-token="tabs pane" className="border-border flex border-b">
      <button type="button" role="tab" aria-selected="true" className="flex h-7 items-center gap-1 px-2 text-small">
        <IconChip kind="doc" size={12} />
        doctor
        <span className="bg-amber-500 size-1.5 rounded-pill" aria-label="Unsaved" />
        <span className="text-muted-foreground" aria-hidden>
          ×
        </span>
      </button>
    </div>
  );
}
