import { ChevronRightIcon } from "lucide-react";
import { useState, type ReactNode } from "react";

import { cn } from "@/lib/utils";

export function Chip({
  children,
  tone = "neutral",
  title,
}: {
  children: ReactNode;
  tone?: "neutral" | "ok" | "warn" | "danger" | "live";
  title?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-1.5 py-px font-mono text-[10px] leading-4",
        tone === "neutral" && "border-border text-muted-foreground",
        tone === "ok" && "border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
        tone === "warn" && "border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-300",
        tone === "danger" && "border-red-500/40 bg-red-500/10 text-red-700 dark:text-red-300",
        tone === "live" && "border-primary/40 bg-primary/10 text-primary",
      )}
      title={title}
    >
      {children}
    </span>
  );
}

export function Section({
  id,
  title,
  count,
  children,
  defaultOpen = true,
  collapsible = true,
  updatedAt,
}: {
  id: string;
  title: string;
  count?: number | string;
  children: ReactNode;
  defaultOpen?: boolean;
  collapsible?: boolean;
  updatedAt?: number;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <section
      className="border-border/70 border-b last:border-b-0"
      data-section={id}
      data-updated-at={updatedAt}
    >
      <button
        aria-expanded={collapsible ? open : undefined}
        className={cn(
          "sticky top-0 z-10 flex w-full items-center gap-1.5 bg-background/95 px-3 py-1.5 text-left backdrop-blur",
          collapsible && "hover:bg-muted/40",
        )}
        disabled={!collapsible}
        onClick={() => collapsible && setOpen((value) => !value)}
        type="button"
      >
        {collapsible ? (
          <ChevronRightIcon
            className={cn("size-3 text-muted-foreground transition-transform", open && "rotate-90")}
          />
        ) : (
          <span className="size-3" />
        )}
        <span className="text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          {title}
        </span>
        {count !== undefined ? <Chip>{count}</Chip> : null}
      </button>
      {open ? <div className="flex flex-col gap-1.5 px-3 pb-3">{children}</div> : null}
    </section>
  );
}

export function Row({
  label,
  children,
  mono = false,
  title,
}: {
  label: string;
  children: ReactNode;
  mono?: boolean;
  title?: string;
}) {
  return (
    <div className="flex min-w-0 items-baseline gap-2 text-[12px] leading-5">
      <span className="w-[92px] shrink-0 text-muted-foreground">{label}</span>
      <span
        className={cn("min-w-0 flex-1 break-words", mono && "font-mono text-[11px]")}
        title={title}
      >
        {children}
      </span>
    </div>
  );
}

export function NotSet({ children = "not set" }: { children?: ReactNode }) {
  return <span className="text-muted-foreground/70 italic">{children}</span>;
}

export function UnavailableRow({ section, reason }: { section: string; reason?: string }) {
  return (
    <div
      className="rounded-sm border border-amber-500/30 bg-amber-500/5 px-2 py-1.5 text-[11px] text-amber-800 dark:text-amber-200"
      data-unavailable={section}
      role="status"
    >
      <span className="font-medium">{section}</span> unavailable
      {reason ? `: ${reason}` : ""}
    </div>
  );
}

export function List({
  items,
  render,
  empty,
}: {
  items: unknown[];
  render: (item: never, index: number) => ReactNode;
  empty: ReactNode;
}) {
  if (items.length === 0) return <p className="text-[12px] text-muted-foreground">{empty}</p>;
  return (
    <ul className="flex flex-col gap-1">
      {items.map((item, index) => (
        <li key={index} className="min-w-0">
          {render(item as never, index)}
        </li>
      ))}
    </ul>
  );
}
