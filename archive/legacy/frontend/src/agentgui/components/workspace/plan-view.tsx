"use client";

import { cn } from "@/agentgui/lib/utils";
import type { Artifact, PlanPayload } from "@/agentgui/types/workspace";
import { CheckIcon, CircleIcon, LoaderCircleIcon } from "lucide-react";
import { useMemo } from "react";

export function PlanView({ artifact }: { artifact: Artifact }) {
  const plan = useMemo<PlanPayload>(() => {
    try {
      const parsed = JSON.parse(artifact.content) as PlanPayload;
      if (Array.isArray(parsed.steps)) return parsed;
    } catch {
      /* fall through */
    }
    return { title: artifact.title, steps: [] };
  }, [artifact.content, artifact.title]);

  const done = plan.steps.filter((step) => step.status === "done").length;
  const progress = plan.steps.length
    ? Math.round((done / plan.steps.length) * 100)
    : 0;

  return (
    <div className="h-full overflow-auto p-5">
      <div className="mx-auto max-w-xl">
        <div className="mb-1 flex items-baseline justify-between gap-3">
          <h2 className="text-[14px] font-semibold tracking-tight">
            {plan.title}
          </h2>
          <span className="font-mono text-[10.5px] text-muted-foreground">
            {done}/{plan.steps.length} done
          </span>
        </div>
        {plan.description && (
          <p className="text-[12px] text-muted-foreground">{plan.description}</p>
        )}

        <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-surface-sunken">
          <div
            className="accent-gradient h-full rounded-full transition-[width] duration-300"
            style={{ width: `${progress}%` }}
          />
        </div>

        <div className="mt-4 space-y-0">
          {plan.steps.map((step, index) => (
            <div key={step.id} className="relative flex gap-3 pb-4 last:pb-0">
              {index < plan.steps.length - 1 && (
                <span className="absolute top-5 left-[9px] h-full w-px bg-border" />
              )}
              <span
                className={cn(
                  "relative mt-0.5 flex size-[19px] shrink-0 items-center justify-center rounded-full border",
                  step.status === "done" &&
                    "border-transparent bg-positive text-white",
                  step.status === "running" &&
                    "accent-gradient border-transparent text-white",
                  step.status === "pending" &&
                    "border-border bg-surface text-muted-foreground"
                )}
              >
                {step.status === "done" ? (
                  <CheckIcon className="size-2.5" />
                ) : step.status === "running" ? (
                  <LoaderCircleIcon className="size-2.5 animate-spin" />
                ) : (
                  <CircleIcon className="size-2" />
                )}
              </span>
              <div className="min-w-0 flex-1 pt-px">
                <p
                  className={cn(
                    "text-[12.5px] leading-snug",
                    step.status === "done" &&
                      "text-muted-foreground line-through decoration-border",
                    step.status === "running" && "font-medium",
                    step.status === "pending" && "text-foreground/85"
                  )}
                >
                  {step.label}
                </p>
                <p className="mt-0.5 font-mono text-[9.5px] tracking-wide text-muted-foreground/70 uppercase">
                  {step.status}
                </p>
              </div>
            </div>
          ))}
        </div>

        {plan.steps.length === 0 && (
          <p className="mt-4 text-[12px] text-muted-foreground">
            This plan has no steps yet.
          </p>
        )}
      </div>
    </div>
  );
}
