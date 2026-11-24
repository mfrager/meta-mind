"use client";

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/agentgui/components/ui/collapsible";
import { Badge } from "@/agentgui/components/ui/badge";
import { Trace } from "@/agentgui/components/ai-elements/trace";
import { cn } from "@/agentgui/lib/utils";
import type { PassPart } from "@/agentgui/types/workspace";
import {
  BrainIcon,
  CheckIcon,
  ChevronDownIcon,
  CircleAlertIcon,
  CircleCheckIcon,
  LoaderCircleIcon,
  ShieldCheckIcon,
  SparklesIcon,
  XIcon,
} from "lucide-react";
import { useEffect, useState } from "react";

const PASS_LABEL: Record<PassPart["pass"], string> = {
  interpret: "Thinking",
  process: "Working",
  verify: "Verifying",
  think: "Sub-agent",
};

const PASS_HINT: Record<PassPart["pass"], string> = {
  interpret: "Reading the request and choosing an approach",
  process: "Doing the work",
  verify: "Checking the result against the request",
  think: "A focused sub-agent is reasoning",
};

function PassIcon({ pass }: { pass: PassPart["pass"] }) {
  switch (pass) {
    case "interpret":
      return <BrainIcon className="size-3.5" />;
    case "process":
      return <SparklesIcon className="size-3.5" />;
    case "verify":
      return <ShieldCheckIcon className="size-3.5" />;
    case "think":
      return <SparklesIcon className="size-3.5" />;
  }
}

function StatusMark({ part }: { part: PassPart }) {
  if (part.status === "running") {
    return <LoaderCircleIcon className="size-3 animate-spin text-primary" />;
  }
  if (part.status === "failed") {
    return <CircleAlertIcon className="size-3 text-destructive" />;
  }
  return <CircleCheckIcon className="size-3 text-positive" />;
}

function BudgetLine({ budget }: { budget: unknown }) {
  if (!budget || typeof budget !== "object") return null;
  const value = budget as { class?: unknown; tokens?: unknown; rounds?: unknown };
  if (typeof value.class !== "string" && typeof value.tokens !== "number") {
    return null;
  }
  return (
    <span>
      budget {typeof value.class === "string" ? value.class : "?"}
      {typeof value.tokens === "number"
        ? ` · ${value.tokens.toLocaleString()} tok left`
        : ""}
    </span>
  );
}

function CheckRow({
  check,
}: {
  check: PassPart["checks"][number];
}) {
  return (
    <div className="flex items-start gap-2 rounded-md border border-border/60 bg-surface/60 px-2 py-1.5">
      {check.pass ? (
        <CheckIcon className="mt-0.5 size-3 shrink-0 text-positive" />
      ) : (
        <XIcon className="mt-0.5 size-3 shrink-0 text-destructive" />
      )}
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[11.5px] font-medium">{check.label}</span>
          <span className="font-mono text-[10px] text-muted-foreground/70">
            {Math.round(check.confidence * 100)}%
          </span>
          {check.backend && (
            <Badge
              className="h-3.5 rounded px-1 font-mono text-[9px] font-normal text-muted-foreground"
              variant="secondary"
            >
              {check.backend}
            </Badge>
          )}
        </div>
        {check.reasoning && (
          <p className="mt-0.5 whitespace-pre-wrap text-[11px] leading-relaxed text-muted-foreground">
            {check.reasoning}
          </p>
        )}
      </div>
    </div>
  );
}

/**
 * One pass of a three-pass turn.
 *
 * This is deliberately not a `MessageResponse`: the prose inside a pass is the
 * model thinking aloud, not the assistant's answer. It is muted, collapsed by
 * default once the pass ends, and labelled so it can never be read as a reply.
 */
export function PassBlock({ part }: { part: PassPart }) {
  const running = part.status === "running";
  const [open, setOpen] = useState(running);

  // A pass is readable while it works and gets out of the way once it succeeds.
  // A failed pass stays open: the error is the useful part, and collapsing it
  // would make a failed first pass look like the turn did nothing at all.
  useEffect(() => {
    if (part.status === "done") setOpen(false);
  }, [part.status]);
  const tokenTotal = part.rounds.reduce(
    (total, round) => total + (round.tokensIn ?? 0) + (round.tokensOut ?? 0),
    0
  );

  return (
    <Collapsible
      className={cn(
        "rounded-xl border bg-surface-muted/35",
        part.status === "failed" ? "border-destructive/25" : "border-border/70"
      )}
      onOpenChange={setOpen}
      open={open}
    >
      <CollapsibleTrigger className="flex w-full items-center gap-2 px-3 py-2 text-left">
        <span
          className={cn(
            "flex size-5 shrink-0 items-center justify-center rounded-md border",
            part.status === "failed"
              ? "border-destructive/25 bg-destructive/8 text-destructive"
              : "border-border bg-surface text-muted-foreground"
          )}
        >
          <PassIcon pass={part.pass} />
        </span>
        <span className="text-[12px] font-medium">{PASS_LABEL[part.pass]}</span>
        <span className="text-[10.5px] text-muted-foreground">
          {PASS_HINT[part.pass]}
        </span>
        <span className="ml-auto flex items-center gap-1.5">
          {part.attempt > 1 && (
            <Badge
              className="h-4 rounded px-1 font-mono text-[9px] font-normal"
              variant="secondary"
            >
              attempt {part.attempt}
            </Badge>
          )}
          {tokenTotal > 0 && (
            <span className="font-mono text-[9.5px] text-muted-foreground/70">
              {tokenTotal.toLocaleString()} tok
            </span>
          )}
          <StatusMark part={part} />
          <ChevronDownIcon className="size-3 text-muted-foreground transition-transform [[data-state=open]>&]:rotate-180" />
        </span>
      </CollapsibleTrigger>

      <CollapsibleContent className="space-y-2 border-t border-border/60 px-3 py-2">
        {Boolean(part.finished || part.budget) && (
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 font-mono text-[10px] text-muted-foreground/70">
            {part.finished && (
              <span>
                {part.pass} · {part.finished}
              </span>
            )}
            <BudgetLine budget={part.budget} />
          </div>
        )}

        {part.rounds.map((round) => (
          <div className="space-y-1" key={round.round}>
            <div className="flex items-center gap-1.5">
              <span className="font-mono text-[9.5px] uppercase tracking-wide text-muted-foreground/70">
                round {round.round}
              </span>
              {round.model && (
                <span className="font-mono text-[9.5px] text-muted-foreground/60">
                  {round.model}
                </span>
              )}
              {round.streaming && (
                <LoaderCircleIcon className="size-2.5 animate-spin text-primary" />
              )}
              {round.outcome && (
                <span
                  className={cn(
                    "font-mono text-[9.5px]",
                    /error|protocol/.test(round.outcome)
                      ? "text-destructive"
                      : "text-muted-foreground/70"
                  )}
                >
                  {round.outcome}
                </span>
              )}
              {round.latencyMs !== undefined && (
                <span className="ml-auto font-mono text-[9.5px] text-muted-foreground/60">
                  {round.latencyMs} ms
                </span>
              )}
            </div>
            {round.text && (
              <div className="max-h-72 overflow-auto rounded-md border border-border/50 bg-surface/70 px-2 py-1.5">
                <p className="whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-muted-foreground">
                  {round.text}
                </p>
              </div>
            )}
            {round.error && (
              <p className="whitespace-pre-wrap text-[11px] text-destructive">
                {round.error}
              </p>
            )}
          </div>
        ))}

        {part.tools.length > 0 && <Trace steps={part.tools} />}

        {part.checks.length > 0 && (
          <div className="space-y-1">
            {part.checks.map((check, index) => (
              <CheckRow check={check} key={`${check.label}-${index}`} />
            ))}
          </div>
        )}

        {part.retry && (
          <p className="rounded-md border border-caution/30 bg-caution/8 px-2 py-1.5 text-[11px] text-muted-foreground">
            Retrying as attempt {part.retry.nextAttempt}: {part.retry.reason}
          </p>
        )}

        {running && part.rounds.length === 0 && (
          <p className="text-[11px] text-muted-foreground">Starting…</p>
        )}
      </CollapsibleContent>
    </Collapsible>
  );
}
