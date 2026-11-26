import type { ReactNode } from "react";

import { BudgetBody } from "@/components/docs/viewers.tsx";
import { StatusChip } from "@/components/kit/data.tsx";
import { useStateBoard, type StateBoard } from "@/lib/api/state.ts";
import { useSideStore } from "@/lib/store/side.ts";

export function StateBoard() {
  const revision = useSideStore((state) => state.revision);
  const lastAction = useSideStore((state) => state.lastAction);
  const query = useStateBoard(revision);
  const board = query.data;

  return (
    <div data-state-board className="flex flex-col gap-3 p-4">
      {query.isPending ? <p className="text-body">Checking the kernel…</p> : null}
      {query.isError ? (
        <p className="text-body text-red-700 dark:text-red-300">
          {query.error instanceof Error ? query.error.message : "The state board did not load."}
        </p>
      ) : null}
      {board ? (
        <>
          <KernelCard board={board} />
          <BeingCard board={board} />
          <GoalsCard board={board} />
          <BudgetsCard board={board} />
          <LastActionCard action={lastAction} />
        </>
      ) : null}
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-border bg-card rounded-xs border">
      <h2 className="text-h3 border-border border-b px-3 py-2">{title}</h2>
      <div className="flex flex-col gap-2 p-3">{children}</div>
    </section>
  );
}

function KernelCard({ board }: { board: StateBoard }) {
  const kernel = board.kernel;
  const tone = kernel.state === "ok" ? "ok" : kernel.state === "degraded" ? "degraded" : "unavailable";
  return (
    <Section title="Kernel">
      <p data-kernel-line className="font-mono text-value">
        kernel {kernel.state}
        {kernel.reason ? ` — ${kernel.reason}` : ""}
      </p>
      <StatusChip tone={tone} label={kernel.state} reason={kernel.reason ?? undefined} />
      {kernel.services.map((service) => (
        <p key={service.name} className="text-small">
          {service.name} {service.state}
          {service.reason ? ` — ${service.reason}` : ""}
        </p>
      ))}
      {kernel.ok_services ? <p className="text-small text-muted-foreground">{kernel.ok_services}</p> : null}
      {kernel.llm_reason ? <p className="text-small text-muted-foreground">{kernel.llm_reason}</p> : null}
    </Section>
  );
}

function BeingCard({ board }: { board: StateBoard }) {
  const being = board.being;
  if (!being.ok) {
    return (
      <Section title="Being">
        <p className="text-body">{being.reason}</p>
      </Section>
    );
  }
  return (
    <Section title="Being">
      <p className="text-body">{being.self_description}</p>
      <p className="font-mono text-value">version {being.version}</p>
      <p className="text-small">{being.invariants.join(", ")}</p>
      <p className="text-small">{being.goal_count} goals</p>
      <p className="font-mono text-value">
        warmth {being.voice.warmth.toFixed(2)} · directness {being.voice.directness.toFixed(2)} · curiosity{" "}
        {being.voice.curiosity.toFixed(2)} · playfulness {being.voice.playfulness.toFixed(2)} · formality{" "}
        {being.voice.formality.toFixed(2)}
      </p>
    </Section>
  );
}

function GoalsCard({ board }: { board: StateBoard }) {
  const goals = board.goals.goals ?? [];
  return (
    <Section title="Goals">
      {goals.length === 0 ? <p className="text-body">No goals are recorded.</p> : null}
      <ul className="flex flex-col gap-1">
        {goals.map((goal) => (
          <li key={goal.id} className="flex items-baseline justify-between gap-3">
            <span className="text-body">{goal.description}</span>
            <span className="font-mono text-value">{goal.priority}</span>
          </li>
        ))}
      </ul>
    </Section>
  );
}

function BudgetsCard({ board }: { board: StateBoard }) {
  const rows = board.budgets.rows ?? [];
  return (
    <Section title="Budgets">
      {rows.length === 0 ? (
        <p className="text-body">{board.budgets.reason ?? "no budget rows are recorded"}</p>
      ) : (
        <BudgetBody rows={rows} />
      )}
    </Section>
  );
}

function LastActionCard({ action }: { action: { name: string; ok: boolean; summary: string } | null }) {
  return (
    <Section title="Last action">
      {action ? (
        <p className="text-body">
          {action.name} {action.ok ? "ok" : "failed"}
          {action.summary ? ` — ${action.summary}` : ""}
        </p>
      ) : (
        <p className="text-small text-muted-foreground">No tool has run in this session.</p>
      )}
    </Section>
  );
}
