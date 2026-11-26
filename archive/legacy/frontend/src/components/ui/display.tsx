import type { HTMLAttributes, ReactNode } from "react";

import { cn } from "@/lib/utils.ts";

export function Badge({ children, tone = "slate" }: { children: ReactNode; tone?: "slate" | "azure" | "green" | "amber" | "red" }) {
  const tones = {
    slate: "border-slate-200 bg-slate-100 text-slate-700",
    azure: "border-azure-200 bg-azure-100 text-azure-800",
    green: "border-green-200 bg-green-100 text-green-800",
    amber: "border-amber-200 bg-amber-100 text-amber-800",
    red: "border-red-200 bg-red-100 text-red-800",
  };
  return (
    <span data-token="badge" className={cn("micro-label rounded-pill border px-2 py-0.5", tones[tone])}>
      {children}
    </span>
  );
}

export function Avatar({ name, src }: { name: string; src?: string }) {
  const initials = name.slice(0, 2).toUpperCase();
  return src ? (
    <img data-token="avatar" alt={name} src={src} className="size-6 rounded-pill object-cover" />
  ) : (
    <span data-token="avatar" role="img" aria-label={name} className="bg-azure-100 text-azure-800 grid size-6 place-items-center rounded-pill text-micro">
      {initials}
    </span>
  );
}

export function AvatarStack({ names }: { names: string[] }) {
  return (
    <div data-token="avatar-stack" className="flex" aria-label={names.join(", ")}>
      {names.map((name) => (
        <span key={name} className="-ml-1 first:ml-0">
          <Avatar name={name} />
        </span>
      ))}
    </div>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd data-token="kbd" className="border-border bg-muted rounded-xs border px-1 font-mono text-value">
      {children}
    </kbd>
  );
}

export function Separator({ orientation = "horizontal" }: { orientation?: "horizontal" | "vertical" }) {
  return (
    <div
      data-token="separator"
      role="separator"
      aria-orientation={orientation}
      className={orientation === "vertical" ? "bg-border w-px self-stretch" : "bg-border h-px w-full"}
    />
  );
}

export function Skeleton({ className }: { className?: string }) {
  return <div data-token="skeleton" className={cn("bg-muted motion-safe:animate-pulse rounded-xs", className)} aria-hidden />;
}

export function Spinner({ label = "Loading" }: { label?: string }) {
  return (
    <span data-token="spinner" role="status" className="text-small text-muted-foreground inline-flex items-center gap-2">
      <span className="border-azure-500 size-3 motion-safe:animate-spin rounded-pill border-2 border-t-transparent" aria-hidden />
      {label}
    </span>
  );
}

export function Progress({ value, label }: { value: number; label: string }) {
  return (
    <div data-token="progress" role="progressbar" aria-label={label} aria-valuenow={value} aria-valuemin={0} aria-valuemax={100}>
      <div className="bg-muted h-1.5 overflow-hidden rounded-pill">
        <div className="bg-azure-500 h-full" style={{ width: `${value}%` }} />
      </div>
    </div>
  );
}

export function Meter({ value, label, min = 0, max = 100 }: { value: number; label: string; min?: number; max?: number }) {
  return (
    <div data-token="meter" role="meter" aria-label={label} aria-valuenow={value} aria-valuemin={min} aria-valuemax={max}>
      <div className="bg-muted h-2 overflow-hidden rounded-xs">
        <div className={cn("h-full", value < 10 ? "bg-red-500" : value < 30 ? "bg-amber-500" : "bg-green-500")} style={{ width: `${value}%` }} />
      </div>
    </div>
  );
}

export function VisuallyHidden({ children }: { children: ReactNode }) {
  return <span className="sr-only">{children}</span>;
}

export function ScrollArea({ children, label, className }: HTMLAttributes<HTMLDivElement> & { label: string }) {
  return (
    <div data-token="scroll-area" tabIndex={0} aria-label={label} className={cn("pane-scroll max-h-40 overflow-auto", className)}>
      {children}
    </div>
  );
}

export function Tooltip({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span data-token="tooltip" className="relative inline-flex" title={label}>
      {children}
      <span role="tooltip" className="border-border bg-popover elevation-1 text-small pointer-events-none absolute top-full left-0 z-10 mt-1 hidden rounded-xs border px-2 py-1 group-hover:block">
        {label}
      </span>
    </span>
  );
}

export function HoverCard({ title, body, children }: { title: string; body: string; children: ReactNode }) {
  return (
    <span data-token="hover-card" className="group relative inline-flex">
      {children}
      <span className="border-border bg-popover elevation-1 absolute top-full left-0 z-10 mt-1 hidden w-56 rounded-xs border p-2 group-hover:block">
        <span className="text-h3 block">{title}</span>
        <span className="text-small text-muted-foreground block">{body}</span>
      </span>
    </span>
  );
}
