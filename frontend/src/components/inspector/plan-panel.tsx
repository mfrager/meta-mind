import { useQuery } from "@tanstack/react-query";
import { ChevronRightIcon } from "lucide-react";
import { useState } from "react";

import { controlProcess, fetchPlan, fetchPlans, fetchProcesses } from "@/lib/api/inspector.ts";
import type { PlanView, ProcessView } from "@/lib/wire/inspector.ts";
import { ago, errorMessage, shortId } from "@/lib/inspector/format.ts";
import { cn } from "@/lib/utils";

import { Chip, NotSet, UnavailableRow } from "./section.tsx";

function statusTone(status: string) {
  if (status === "done" || status === "completed" || status === "active") return "ok" as const;
  if (status === "failed" || status === "abandoned") return "danger" as const;
  if (status === "in_progress" || status === "running" || status === "waiting") return "warn" as const;
  return "neutral" as const;
}

function StepList({ plan }: { plan: PlanView }) {
  return (
    <ol className="flex flex-col gap-1">
      {plan.steps.map((step, index) => (
        <li className="border-border rounded-sm border p-1.5" key={step.id}>
          <div className="flex items-center gap-1.5">
            <span className="w-4 shrink-0 font-mono text-[10px] text-muted-foreground">{index + 1}</span>
            <span className="min-w-0 flex-1 truncate text-[12px]">{step.title}</span>
            <Chip tone={statusTone(step.status)}>{step.status}</Chip>
            <span className="font-mono text-[10px] text-muted-foreground">
              {step.attempts} att · {step.runs} runs
            </span>
          </div>
          {step.dependsOn.length ? (
            <p className="mt-0.5 pl-5 font-mono text-[10px] text-muted-foreground">
              depends on {step.dependsOn.join(", ")}
            </p>
          ) : null}
          {step.detail ? <p className="mt-0.5 pl-5 text-[10px] text-muted-foreground">{step.detail}</p> : null}
          {step.lastError ? (
            <p className="mt-0.5 pl-5 text-[10px] text-red-700 dark:text-red-300">{step.lastError}</p>
          ) : null}
          {step.agentRunId ? (
            <p className="mt-0.5 pl-5 font-mono text-[10px] text-muted-foreground">
              run {shortId(step.agentRunId)}
            </p>
          ) : null}
        </li>
      ))}
    </ol>
  );
}

function ProcessRow({
  process,
  onControl,
}: {
  process: ProcessView;
  onControl: (id: string, action: "pause" | "resume" | "cancel") => void;
}) {
  const steps = Object.entries(process.steps);
  const done = steps.filter(([, step]) => step.status === "done").length;
  const live = process.status === "running" || process.status === "waiting" || process.status === "queued";
  return (
    <div className="border-border rounded-sm border p-1.5">
      <div className="flex items-center gap-1.5">
        <span className="min-w-0 flex-1 truncate font-mono text-[10px]">{shortId(process.id)}</span>
        <Chip tone={statusTone(process.status)}>{process.status}</Chip>
      </div>
      <p className="mt-0.5 font-mono text-[10px] text-muted-foreground">
        {done}/{steps.length} steps · v{process.templateVersion} · {process.tokensIn + process.tokensOut} tokens
      </p>
      {process.waiting ? (
        <p className="mt-0.5 text-[10px] text-amber-700 dark:text-amber-300">
          waiting: {process.waiting.detail || process.waiting.kind}
        </p>
      ) : null}
      {process.error ? (
        <p className="mt-0.5 text-[10px] text-red-700 dark:text-red-300">{process.error}</p>
      ) : null}
      {process.result ? <p className="mt-0.5 text-[10px] text-muted-foreground">{process.result}</p> : null}
      {live || process.status === "paused" ? (
        <div className="mt-1 flex gap-1">
          {process.status === "paused" || process.status === "waiting" ? (
            <button
              className="rounded-sm border border-border px-1.5 py-0.5 text-[10px] hover:bg-muted/50"
              onClick={() => onControl(process.id, "resume")}
              type="button"
            >
              Resume
            </button>
          ) : (
            <button
              className="rounded-sm border border-border px-1.5 py-0.5 text-[10px] hover:bg-muted/50"
              onClick={() => onControl(process.id, "pause")}
              type="button"
            >
              Pause
            </button>
          )}
          <button
            className="rounded-sm border border-border px-1.5 py-0.5 text-[10px] text-red-700 hover:bg-muted/50 dark:text-red-300"
            onClick={() => onControl(process.id, "cancel")}
            type="button"
          >
            Cancel
          </button>
        </div>
      ) : null}
    </div>
  );
}

export function PlanPanel({ sessionId }: { sessionId: string | null }) {
  const plans = useQuery({ queryKey: ["inspector", "plans"], queryFn: fetchPlans });
  const processes = useQuery({
    queryKey: ["inspector", "processes", sessionId],
    queryFn: () => fetchProcesses(sessionId ?? undefined),
    enabled: Boolean(sessionId),
  });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [version, setVersion] = useState<number | null>(null);
  const detail = useQuery({
    queryKey: ["inspector", "plan", selectedId],
    queryFn: () => fetchPlan(selectedId as string),
    enabled: Boolean(selectedId),
  });
  const plan = detail.data?.plan ?? plans.data?.items.find((item) => item.id === selectedId) ?? null;
  const versions = detail.data?.versions ?? [];
  const visibleVersion = version
    ? versions.find((entry) => entry.version === version)
    : null;
  const planProcesses =
    plan?.processUri && processes.data?.enabled
      ? processes.data.processes.filter((entry) => entry.templateUri === plan.processUri)
      : [];

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-panel="plan">
      <div className="border-border/70 flex items-center gap-2 border-b px-3 py-2">
        <span className="flex-1 text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          Plan
        </span>
        <Chip>{plans.data?.items.length ?? 0} registered</Chip>
        <Chip tone={plans.data?.monitor ? "live" : "neutral"}>
          monitor {plans.data?.monitor ? "on" : "off"}
        </Chip>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {plans.isError ? <UnavailableRow section="plans" reason={errorMessage(plans.error)} /> : null}
        {plans.isPending ? <p className="text-[12px] text-muted-foreground">Loading plans…</p> : null}
        {plans.data && plans.data.items.length === 0 ? (
          <p className="text-[12px] text-muted-foreground">No plans are registered for this kernel.</p>
        ) : null}
        <div className="mb-2 flex flex-col gap-0.5">
          {plans.data?.items.map((item) => (
            <button
              aria-pressed={selectedId === item.id}
              className={cn(
                "flex min-w-0 items-center gap-1.5 rounded-sm px-1.5 py-1 text-left",
                selectedId === item.id ? "bg-surface shadow-elev-1" : "hover:bg-muted/50",
              )}
              key={item.id}
              onClick={() => {
                setSelectedId(item.id);
                setVersion(null);
              }}
              type="button"
            >
              <ChevronRightIcon className="size-3 shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate text-[12px]">{item.goal}</span>
              <Chip tone={statusTone(item.status)}>{item.status}</Chip>
              <span className="font-mono text-[10px] text-muted-foreground">v{item.revision}</span>
            </button>
          ))}
        </div>

        {selectedId && detail.isError ? (
          <UnavailableRow section="plan detail" reason={errorMessage(detail.error)} />
        ) : null}
        {plan ? (
          <div className="flex flex-col gap-2">
            <div className="border-border rounded-sm border p-2">
              <p className="text-[13px] font-medium">{plan.goal}</p>
              <p className="mt-0.5 font-mono text-[10px] text-muted-foreground">
                {plan.id} · rev {plan.revision} · updated {ago(plan.updatedAt)}
              </p>
              <div className="mt-1 flex flex-wrap gap-1">
                <Chip tone={statusTone(plan.status)}>{plan.status}</Chip>
                <Chip>{plan.format}</Chip>
                <Chip>{plan.steps.length} steps</Chip>
                <Chip>version {plan.currentVersion}</Chip>
                <Chip>owner {plan.owner}</Chip>
                <Chip tone={plan.monitor ? "live" : "neutral"}>{plan.monitor ? "monitored" : "not monitored"}</Chip>
                {plan.neverExpires ? <Chip tone="ok">durable</Chip> : null}
                {plan.expiresAt ? <Chip tone="warn">expires {ago(plan.expiresAt)}</Chip> : null}
                {plan.processUri ? <Chip tone="live">process</Chip> : null}
              </div>
              {plan.description ? (
                <p className="mt-1 text-[11px] text-muted-foreground">{plan.description}</p>
              ) : null}
              <p className="mt-1 text-[11px]">
                current step: {plan.currentStep ?? <NotSet>not set</NotSet>}
              </p>
            </div>
            <StepList plan={plan} />
            {plan.processUri ? (
              <div className="border-border rounded-sm border p-2">
                <div className="flex items-center gap-1">
                  <span className="flex-1 text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
                    Process
                  </span>
                  <Chip>{processes.data?.enabled ? `${planProcesses.length} runs` : "pipeline off"}</Chip>
                </div>
                {processes.isError ? (
                  <p className="mt-1 text-[10px] text-muted-foreground">
                    {errorMessage(processes.error)}
                  </p>
                ) : null}
                <div className="mt-1 flex flex-col gap-1">
                  {planProcesses.map((process) => (
                    <ProcessRow
                      key={process.id}
                      onControl={(id, action) => {
                        void controlProcess(id, action).then(() => processes.refetch());
                      }}
                      process={process}
                    />
                  ))}
                  {processes.data?.enabled && planProcesses.length === 0 ? (
                    <p className="text-[10px] text-muted-foreground">
                      This plan has a template but no process run yet.
                    </p>
                  ) : null}
                </div>
              </div>
            ) : null}
            {versions.length ? (
              <div className="border-border rounded-sm border p-2">
                <div className="flex items-center gap-1">
                  <span className="flex-1 text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
                    Versions
                  </span>
                  {versions.map((entry) => (
                    <button
                      aria-pressed={version === entry.version}
                      className={cn(
                        "rounded-sm px-1.5 py-0.5 font-mono text-[10px]",
                        version === entry.version ? "bg-surface shadow-elev-1" : "text-muted-foreground hover:text-foreground",
                      )}
                      key={entry.version}
                      onClick={() => setVersion(entry.version === version ? null : entry.version)}
                      type="button"
                    >
                      v{entry.version}
                    </button>
                  ))}
                </div>
                {visibleVersion ? (
                  <pre className="mt-1.5 max-h-56 overflow-auto whitespace-pre-wrap rounded-sm bg-muted/40 p-2 font-mono text-[10px]">
                    {visibleVersion.body || "This version has no body recorded."}
                  </pre>
                ) : (
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    Select a version to inspect its body.
                  </p>
                )}
              </div>
            ) : null}
          </div>
        ) : null}
        {!selectedId && plans.data?.items.length ? (
          <p className="text-[11px] text-muted-foreground">Select a plan to inspect versions and steps.</p>
        ) : null}
        {sessionId ? (
          <p className="mt-2 font-mono text-[10px] text-muted-foreground">
            registry scan for session {shortId(sessionId)}
          </p>
        ) : null}
      </div>
    </div>
  );
}
