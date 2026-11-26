"use client";

import { Badge } from "@/agentgui/components/ui/badge";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/agentgui/components/ui/collapsible";
import { cn } from "@/agentgui/lib/utils";
import type { StepStatus, TraceStep } from "@/agentgui/types/workspace";
import {
  BotIcon,
  ChevronDownIcon,
  CircleCheckIcon,
  CircleXIcon,
  LoaderCircleIcon,
} from "lucide-react";
import { useState, type ComponentProps } from "react";

import { TraceStepItem } from "./trace";

export type SubagentProps = ComponentProps<"div"> & {
  name: string;
  model: string;
  status: StepStatus;
  steps?: TraceStep[];
  defaultOpen?: boolean;
};

const statusCopy: Record<StepStatus, string> = {
  pending: "Queued",
  running: "Running",
  done: "Finished",
  error: "Failed",
};

export const Subagent = ({
  name,
  model,
  status,
  steps = [],
  defaultOpen = false,
  className,
  ...props
}: SubagentProps) => {
  const [open, setOpen] = useState(defaultOpen);

  return (
    <div
      className={cn(
        "rounded-xl border border-border/80 bg-surface-muted/40",
        className
      )}
      {...props}
    >
      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger className="flex w-full items-center gap-2 px-3 py-2 text-left">
          <span className="accent-soft flex size-6 items-center justify-center rounded-md border border-border/70">
            <BotIcon className="size-3.5 text-primary" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[13px] font-medium">{name}</span>
            <span className="block truncate font-mono text-[10px] text-muted-foreground">
              {model}
            </span>
          </span>
          <Badge
            variant="secondary"
            className="h-5 gap-1 rounded-md px-1.5 text-[10px] font-normal"
          >
            {status === "running" && (
              <LoaderCircleIcon className="size-2.5 animate-spin text-primary" />
            )}
            {status === "done" && (
              <CircleCheckIcon className="size-2.5 text-positive" />
            )}
            {status === "error" && (
              <CircleXIcon className="size-2.5 text-destructive" />
            )}
            {statusCopy[status]}
          </Badge>
          <ChevronDownIcon
            className={cn(
              "size-3.5 text-muted-foreground transition-transform",
              open && "rotate-180"
            )}
          />
        </CollapsibleTrigger>
        <CollapsibleContent className="overflow-hidden">
          <div className="border-t border-border/70 px-1.5 py-1.5">
            {steps.length === 0 ? (
              <p className="px-2 py-1.5 text-[11px] text-muted-foreground">
                No reported steps yet.
              </p>
            ) : (
              steps.map((step) => <TraceStepItem key={step.id} step={step} />)
            )}
          </div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
};
