"use client";

import { Badge } from "@/agentgui/components/ui/badge";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/agentgui/components/ui/collapsible";
import { cn } from "@/agentgui/lib/utils";
import type { TraceStep } from "@/agentgui/types/workspace";
import {
  BotIcon,
  ChevronDownIcon,
  CircleCheckIcon,
  CircleXIcon,
  ClockIcon,
  ListChecksIcon,
  LoaderCircleIcon,
  SearchIcon,
  WrenchIcon,
} from "lucide-react";
import { useState, type ComponentProps } from "react";

function formatDuration(ms?: number): string | null {
  if (ms === undefined) return null;
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

function StepIcon({ step }: { step: TraceStep }) {
  if (step.status === "running") {
    return <LoaderCircleIcon className="size-3.5 animate-spin text-primary" />;
  }
  if (step.status === "error") {
    return <CircleXIcon className="size-3.5 text-destructive" />;
  }
  if (step.status === "done") {
    return <CircleCheckIcon className="size-3.5 text-positive" />;
  }
  const Icon =
    step.kind === "search"
      ? SearchIcon
      : step.kind === "plan"
        ? ListChecksIcon
        : step.kind === "subagent"
          ? BotIcon
          : WrenchIcon;
  return <Icon className="size-3.5 text-muted-foreground" />;
}

export type TraceStepItemProps = ComponentProps<"div"> & {
  step: TraceStep;
};

export const TraceStepItem = ({
  step,
  className,
  ...props
}: TraceStepItemProps) => {
  const [open, setOpen] = useState(false);
  const hasDetail = Boolean(step.input) || Boolean(step.output);
  const duration = formatDuration(step.durationMs);

  return (
    <div
      className={cn(
        "relative rounded-lg border border-transparent px-2 py-1.5 transition-colors",
        hasDetail && "hover:border-border/70 hover:bg-surface-muted/70",
        className
      )}
      {...props}
    >
      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger
          className="flex w-full items-start gap-2 text-left"
          disabled={!hasDetail}
        >
          <span className="mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-md border border-border/80 bg-surface-muted">
            <StepIcon step={step} />
          </span>
          <span className="min-w-0 flex-1">
            <span className="flex items-baseline gap-2">
              <span className="truncate font-medium text-[13px]">
                {step.title}
              </span>
              <span className="font-mono text-[11px] text-muted-foreground">
                {duration ?? (step.status === "running" ? "running" : "")}
              </span>
            </span>
            {step.description && (
              <span className="mt-0.5 block truncate font-mono text-[11px] text-muted-foreground">
                {step.description}
              </span>
            )}
          </span>
          {hasDetail && (
            <ChevronDownIcon
              className={cn(
                "mt-1 size-3.5 shrink-0 text-muted-foreground transition-transform",
                open && "rotate-180"
              )}
            />
          )}
        </CollapsibleTrigger>

        <CollapsibleContent className="overflow-hidden">
          <div className="mt-2 space-y-2 pl-7">
            {step.input !== undefined && (
              <div className="surface-inset overflow-x-auto px-2.5 py-2">
                <p className="mb-1 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                  Input
                </p>
                <pre className="font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-foreground/80">
                  {typeof step.input === "string"
                    ? step.input
                    : JSON.stringify(step.input, null, 2)}
                </pre>
              </div>
            )}
            {step.output && (
              <div className="overflow-x-auto rounded-lg border border-border bg-surface-muted px-2.5 py-2">
                <p className="mb-1 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
                  Output
                </p>
                <pre className="font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-foreground/80">
                  {step.output}
                </pre>
              </div>
            )}
          </div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
};

export type TraceProps = ComponentProps<"div"> & {
  steps: TraceStep[];
  title?: string;
  defaultOpen?: boolean;
};

export const Trace = ({
  steps,
  title = "Execution trace",
  defaultOpen = true,
  className,
  ...props
}: TraceProps) => {
  const [open, setOpen] = useState(defaultOpen);
  const running = steps.some((step) => step.status === "running");
  const total = steps.reduce((sum, step) => sum + (step.durationMs ?? 0), 0);

  return (
    <div
      className={cn(
        "rounded-xl border border-border/80 bg-surface-muted/45",
        className
      )}
      {...props}
    >
      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger className="flex w-full items-center gap-2 px-3 py-2 text-left">
          <span
            className={cn(
              "flex size-5 items-center justify-center rounded-md",
              running ? "accent-gradient text-white" : "surface-inset"
            )}
          >
            <WrenchIcon className="size-3" />
          </span>
          <span className="flex-1 text-[13px] font-medium">{title}</span>
          <Badge
            variant="secondary"
            className="h-5 rounded-md px-1.5 font-mono text-[10px] font-normal"
          >
            {steps.length} steps
          </Badge>
          {total > 0 && (
            <span className="font-mono text-[10px] text-muted-foreground">
              {formatDuration(total)}
            </span>
          )}
          <ChevronDownIcon
            className={cn(
              "size-3.5 text-muted-foreground transition-transform",
              open && "rotate-180"
            )}
          />
        </CollapsibleTrigger>
        <CollapsibleContent className="overflow-hidden">
          <div className="space-y-0.5 border-t border-border/70 px-1.5 py-1.5">
            {steps.map((step) => (
              <TraceStepItem key={step.id} step={step} />
            ))}
            {running && (
              <div className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-muted-foreground">
                <ClockIcon className="size-3" />
                Working…
              </div>
            )}
          </div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
};
