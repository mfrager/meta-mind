import type { StatusView } from "@/lib/wire/inspector.ts";
import { asList, asRecord, asString, clockAt } from "@/lib/inspector/format.ts";

import { Chip, NotSet } from "./section.tsx";

export function TopicHeader({ view }: { view: StatusView }) {
  const status = view.state.status;
  const phase = asString(status.phase) ?? asString(status.status);
  const intent = asString(status.intent);
  const focus = asRecord(status.focus);
  const focusLabel =
    (focus && (asString(focus.label) ?? asString(focus.id))) ??
    (typeof status.focus === "string" ? status.focus : null);
  const questions = asList(status.questions);
  return (
    <section
      aria-live="polite"
      className="border-border/70 bg-surface/40 border-b px-3 py-2.5"
      data-section="topic"
      data-state-revision={view.stateRevision}
    >
      <div className="flex items-start gap-2">
        <h2 className="text-h2 min-w-0 flex-1 leading-tight" data-topic>
          {view.state.topic ?? <span className="text-muted-foreground italic">topic not set</span>}
        </h2>
        <Chip tone="neutral" title="Conversation depth">
          depth {view.state.depth}
        </Chip>
      </div>
      {intent ? <p className="mt-1 text-[12px] leading-5 text-foreground/85">{intent}</p> : null}
      <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
        <span className="font-mono">origin {view.state.updatedBy}</span>
        <span aria-hidden>·</span>
        <span>updated {clockAt(view.state.updatedAt)}</span>
        {phase ? (
          <>
            <span aria-hidden>·</span>
            <span className="text-foreground/80">phase {phase}</span>
          </>
        ) : null}
        {focusLabel ? (
          <>
            <span aria-hidden>·</span>
            <span className="min-w-0 truncate" title={focusLabel}>
              focus {focusLabel}
              {focus && typeof focus.verified === "boolean" ? (focus.verified ? " ✓" : " unverified") : ""}
            </span>
          </>
        ) : null}
      </div>
      {questions.length ? (
        <div className="mt-1 flex flex-wrap gap-1">
          {questions.map((question) => (
            <span
              className="rounded-sm border border-amber-500/30 bg-amber-500/5 px-1.5 py-0.5 text-[10px] text-amber-800 dark:text-amber-200"
              key={question}
            >
              {question}
            </span>
          ))}
        </div>
      ) : null}
      {!view.state.topic && !intent ? (
        <p className="mt-1 text-[11px] text-muted-foreground">
          The conversation manager has not named this topic. <NotSet>not set</NotSet>
        </p>
      ) : null}
    </section>
  );
}
