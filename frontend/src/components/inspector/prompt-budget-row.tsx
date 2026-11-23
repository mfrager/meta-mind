import type { StatusView } from "@/lib/wire/inspector.ts";
import { clockAt, compactTokens, shortId } from "@/lib/inspector/format.ts";

import { Chip, NotSet } from "./section.tsx";

export function PromptBudgetRow({ view }: { view: StatusView }) {
  const prompt = view.prompt;
  const budget = view.budget;
  const budgetReason = view.unavailable.budget;
  return (
    <div className="border-border/70 flex flex-col gap-0.5 border-b px-3 py-2" data-section="prompt-budget">
      <div className="flex min-w-0 items-center gap-1.5">
        <span className="text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          prompt
        </span>
        {prompt ? (
          <>
            <span className="font-mono text-[11px]">{prompt.strategy}</span>
            {prompt.variant ? <Chip>{prompt.variant}</Chip> : null}
            <span
              className="min-w-0 truncate font-mono text-[10px] text-muted-foreground"
              title={`${prompt.hash} · ${prompt.id}`}
            >
              {prompt.hash ? shortId(prompt.hash) : "hash not set"} · {shortId(prompt.id)}
            </span>
            <span className="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground">
              {prompt.fragments.reduce((sum, fragment) => sum + fragment.chars, 0).toLocaleString()} chars
            </span>
          </>
        ) : (
          <NotSet>not built for this session</NotSet>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          budget
        </span>
        <span className="font-mono text-[11px]">{budget.class || "not set"}</span>
        <Chip>{budget.turnsUsed}/{budget.turnCap || "—"} turns</Chip>
        <Chip>
          {compactTokens(budget.tokensUsed)}/{budget.tokenCap ? compactTokens(budget.tokenCap) : "—"} tokens
        </Chip>
        {budget.estimated ? <Chip tone="warn">estimated</Chip> : null}
        {budgetReason ? (
          <span className="min-w-0 truncate text-[10px] text-amber-700 dark:text-amber-300" title={budgetReason}>
            unavailable: {budgetReason}
          </span>
        ) : (
          <span className="ml-auto font-mono text-[10px] text-muted-foreground">
            updated {clockAt(view.generatedAt)}
          </span>
        )}
      </div>
    </div>
  );
}
