"use client";

import { Button } from "@/agentgui/components/ui/button";
import { cn } from "@/agentgui/lib/utils";
import type { QuestionnairePayload } from "@/agentgui/types/workspace";
import { CheckIcon, HelpCircleIcon } from "lucide-react";
import { useState } from "react";

export type QuestionnaireProps = {
  payload: QuestionnairePayload;
  onSubmit?: (answer: string[]) => void;
  className?: string;
};

/** Agent-issued question with a small set of selectable answers. */
export const Questionnaire = ({
  payload,
  onSubmit,
  className,
}: QuestionnaireProps) => {
  const [selected, setSelected] = useState<string[]>([]);
  const [submitted, setSubmitted] = useState(false);

  const toggle = (id: string) => {
    if (submitted) return;
    setSelected((current) => {
      if (payload.multiSelect) {
        return current.includes(id)
          ? current.filter((entry) => entry !== id)
          : [...current, id];
      }
      return current[0] === id ? [] : [id];
    });
  };

  const submit = () => {
    if (selected.length === 0) return;
    setSubmitted(true);
    onSubmit?.(selected);
  };

  return (
    <div
      className={cn(
        "overflow-hidden rounded-xl border border-border/80 bg-surface shadow-elev-1",
        className
      )}
    >
      <div className="flex items-start gap-2 border-b border-border/70 bg-surface-muted/60 px-3 py-2">
        <HelpCircleIcon className="mt-0.5 size-3.5 shrink-0 text-primary" />
        <p className="text-[13px] font-medium leading-snug">{payload.question}</p>
      </div>

      <div className="space-y-1 p-2">
        {payload.options.map((option) => {
          const active = selected.includes(option.id);
          return (
            <button
              className={cn(
                "flex w-full items-start gap-2 rounded-lg border px-2.5 py-2 text-left transition-colors",
                active
                  ? "border-primary/35 bg-accent/70"
                  : "border-transparent hover:border-border hover:bg-surface-muted"
              )}
              key={option.id}
              onClick={() => toggle(option.id)}
              type="button"
            >
              <span
                className={cn(
                  "mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border",
                  active
                    ? "accent-gradient border-transparent text-white"
                    : "border-input bg-surface"
                )}
              >
                {active && <CheckIcon className="size-2.5" />}
              </span>
              <span className="min-w-0">
                <span className="block text-[13px] font-medium leading-tight">
                  {option.label}
                </span>
                {option.description && (
                  <span className="mt-0.5 block text-[11px] text-muted-foreground">
                    {option.description}
                  </span>
                )}
              </span>
            </button>
          );
        })}
      </div>

      <div className="flex items-center justify-between gap-2 border-t border-border/70 px-3 py-2">
        <span className="text-[11px] text-muted-foreground">
          {submitted
            ? "Answer sent"
            : payload.multiSelect
              ? "Select any that apply"
              : "Choose one"}
        </span>
        <Button
          disabled={selected.length === 0 || submitted}
          onClick={submit}
          size="sm"
          type="button"
        >
          {submitted ? "Sent" : "Submit"}
        </Button>
      </div>
    </div>
  );
};
