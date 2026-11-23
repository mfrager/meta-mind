import { useQuery } from "@tanstack/react-query";
import { ArrowUpRightIcon, CircleDotIcon, ExternalLinkIcon, XIcon } from "lucide-react";

import { fetchStateBoard } from "@/lib/api/inspector.ts";
import type { ActionStatusView, AgentStatusView, StatusValue, StatusView } from "@/lib/wire/inspector.ts";
import {
  ago,
  asBoolean,
  asList,
  asNumber,
  asRecord,
  asString,
  clockAt,
  compactTokens,
  shortId,
  until,
} from "@/lib/inspector/format.ts";
import { useNow } from "@/hooks/use-now.ts";
import { Chip, NotSet, Row, Section, UnavailableRow } from "./section.tsx";

function presenceTone(presence: string) {
  if (presence === "present") return "ok" as const;
  if (presence === "away") return "warn" as const;
  return "neutral" as const;
}

function lifecycleTone(lifecycle: string) {
  if (lifecycle === "active") return "ok" as const;
  if (lifecycle === "ending") return "warn" as const;
  return "neutral" as const;
}

export function SessionBlock({
  view,
  onOpenFiles,
}: {
  view: StatusView;
  onOpenFiles?: () => void;
}) {
  const session = view.session;
  const now = useNow();
  return (
    <Section id="session" title="Session" count={session.lifecycle} updatedAt={session.lastActivityAt}>
      <div className="flex flex-wrap items-center gap-1">
        <Chip tone={lifecycleTone(session.lifecycle)}>{session.lifecycle}</Chip>
        <Chip tone={presenceTone(session.presence)}>{session.presence}</Chip>
        {session.working ? <Chip tone="live">working</Chip> : <Chip>idle</Chip>}
        <Chip>{session.traceCount} traces</Chip>
      </div>
      <Row label="last input">{ago(session.lastUserInputAt, now)}</Row>
      <Row label="last work">{ago(session.lastWorkActivityAt, now)}</Row>
      <Row label="expires">{session.expiresAt ? until(session.expiresAt, now) : "no explicit expiry"}</Row>
      <Row label="traces" mono title={session.traces.join(", ")}>
        {session.traces.length ? session.traces.join(" · ") : <NotSet>none</NotSet>}
      </Row>
      <Row label="main trace" mono>
        {session.mainTraceId ?? <NotSet />}
      </Row>
      <Row label="directory" mono title={session.sessionDir ?? undefined}>
        {session.sessionDir ?? <NotSet>on demand</NotSet>}
      </Row>
      <div className="mt-0.5 flex items-center justify-between gap-2">
        <span className="text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          work catalog
        </span>
        {onOpenFiles ? (
          <button
            className="inline-flex items-center gap-1 text-[11px] text-primary hover:underline"
            onClick={onOpenFiles}
            type="button"
          >
            Files <ExternalLinkIcon className="size-3" />
          </button>
        ) : null}
      </div>
      {session.work.length === 0 ? (
        <p className="text-[12px] text-muted-foreground">No work items are recorded.</p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {session.work.map((item) => (
            <li className="flex min-w-0 items-baseline gap-1.5 text-[12px]" key={item.id}>
              <CircleDotIcon className="size-2.5 shrink-0 text-muted-foreground/60" />
              <span className="min-w-0 flex-1 truncate">{item.title}</span>
              <Chip>{item.kind}</Chip>
              <span className="font-mono text-[10px] text-muted-foreground">{item.status}</span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

export function FrameBlock({ view }: { view: StatusView }) {
  const frames = view.frames;
  const top =
    frames.frames.find((frame) => frame.id === frames.topFrameId) ??
    frames.frames[frames.frames.length - 1];
  const attempt = asNumber(view.state.status.frame_attempt);
  const awaiting = asBoolean(view.state.status.awaiting_user);
  return (
    <Section id="frame" title="Frame" count={frames.frames.length} updatedAt={top?.updatedAt}>
      {top ? (
        <>
          <Row label="top" mono title={top.templateId}>
            {top.templateId}
          </Row>
          <div className="flex flex-wrap items-center gap-1">
            <Chip>{top.kind}</Chip>
            <Chip tone={top.status === "active" ? "live" : "neutral"}>{top.status}</Chip>
            <Chip>attempt {attempt ?? "not set"}</Chip>
            <Chip tone={awaiting ? "warn" : "neutral"}>
              {awaiting === null ? "awaiting unknown" : awaiting ? "awaiting user" : "not awaiting"}
            </Chip>
          </div>
          <Row label="entered">{clockAt(top.enteredAt)}</Row>
          <Row label="expires">{top.expiresAt ? until(top.expiresAt) : <NotSet />}</Row>
          <Row label="stack" mono title={frames.frames.map((frame) => frame.templateId).join(" ▸ ")}>
            {frames.frames.map((frame) => frame.kind || shortId(frame.templateId)).join(" ▸ ")}
          </Row>
          <Row label="utterance" mono>
            {frames.pendingUtterance ? (
              JSON.stringify(frames.pendingUtterance)
            ) : (
              <NotSet>none scheduled</NotSet>
            )}
          </Row>
        </>
      ) : (
        <p className="text-[12px] text-muted-foreground">The frame stack is empty.</p>
      )}
    </Section>
  );
}

export function ConversationBlock({ view }: { view: StatusView }) {
  const phase = asString(view.state.status.phase) ?? asString(view.state.status.status);
  return (
    <Section id="conversation" title="Conversation" count={view.state.revision} updatedAt={view.state.updatedAt}>
      <Row label="state rev" mono>
        {view.state.revision} · by {view.state.updatedBy}
      </Row>
      <Row label="phase">{phase ?? <NotSet>not set</NotSet>}</Row>
      <Row label="intent">{asString(view.state.status.intent) ?? <NotSet>not set</NotSet>}</Row>
      <Row label="turn" mono>
        <NotSet>not exposed by the status view</NotSet>
      </Row>
      <Row label="loop" mono>
        <NotSet>not exposed by the status view</NotSet>
      </Row>
      <Row label="pending">
        {asList(view.state.status.next).length} next · {view.agents.filter((run) => run.active).length} active runs
      </Row>
    </Section>
  );
}

export function MetamindBlock() {
  const query = useQuery({ queryKey: ["inspector", "state-board"], queryFn: fetchStateBoard, retry: 1 });
  const board = query.data;
  return (
    <Section id="metamind" title="Metamind" count={board?.kernel.state}>
      {query.isError ? <UnavailableRow section="metamind" reason="state board request failed" /> : null}
      {board ? (
        <>
          <div className="flex flex-wrap items-center gap-1">
            <Chip tone={board.kernel.state === "ok" ? "ok" : "warn"}>{board.kernel.state}</Chip>
            {board.kernel.services.map((service) => (
              <Chip key={service.name} tone={service.state === "ok" ? "ok" : "warn"} title={service.reason ?? undefined}>
                {service.name} {service.state}
              </Chip>
            ))}
          </div>
          {board.being.ok ? (
            <>
              <Row label="being">
                v{board.being.version} · {board.being.goal_count} goals
              </Row>
              <Row label="voice" mono>
                warmth {board.being.voice.warmth.toFixed(2)} · directness{" "}
                {board.being.voice.directness.toFixed(2)} · curiosity{" "}
                {board.being.voice.curiosity.toFixed(2)}
              </Row>
            </>
          ) : (
            <UnavailableRow section="being" reason={board.being.reason} />
          )}
          <Row label="budgets">
            {board.budgets.rows?.length ?? 0} rows
            {board.budgets.reason ? ` · ${board.budgets.reason}` : ""}
          </Row>
          <Row label="last action">
            {board.last_action ? board.last_action.summary : <NotSet />}
          </Row>
        </>
      ) : null}
      {query.isPending ? <p className="text-[12px] text-muted-foreground">Reading kernel state…</p> : null}
    </Section>
  );
}

export function KnowledgeBlock({ view, onOpen }: { view: StatusView; onOpen?: () => void }) {
  const data = view.data;
  return (
    <Section
      id="knowledge"
      title="Knowledge"
      count={data.indexPending ? `${data.indexPending} pending` : `${data.schemas} schemas`}
      updatedAt={view.generatedAt}
    >
      <Row label="revision" mono>
        {data.dataRevision}
      </Row>
      <Row label="index lag">{data.indexLagMs} ms</Row>
      <Row label="outbox">{data.indexPending ? <span className="text-amber-600">{data.indexPending} pending</span> : "0 pending"}</Row>
      <Row label="schemas">{data.schemas}</Row>
      <Row label="namespaces">{data.namespaces}</Row>
      {onOpen ? (
        <button
          className="inline-flex items-center gap-1 self-start text-[11px] text-primary hover:underline"
          onClick={onOpen}
          type="button"
        >
          Inspect knowledge <ArrowUpRightIcon className="size-3" />
        </button>
      ) : null}
    </Section>
  );
}

export function PlanBlock({ view, onOpen }: { view: StatusView; onOpen?: () => void }) {
  const plan = view.plans.items.find((item) => item.status === "active") ?? view.plans.items[0];
  const done = plan?.steps.filter((step) => step.status === "done" || step.status === "skipped").length ?? 0;
  return (
    <Section id="plan" title="Plan" count={view.plans.items.length} updatedAt={view.generatedAt}>
      {plan ? (
        <>
          <Row label="goal" title={plan.goal}>
            {plan.goal}
          </Row>
          <div className="flex flex-wrap items-center gap-1">
            <Chip tone={plan.status === "active" ? "live" : "neutral"}>{plan.status}</Chip>
            <Chip>rev {plan.revision}</Chip>
            <Chip>
              {done}/{plan.steps.length} steps
            </Chip>
          </div>
          <Row label="current" mono>
            {plan.currentStep ?? <NotSet />}
          </Row>
          <Row label="next" title={plan.steps.find((step) => step.status !== "done")?.title}>
            {plan.steps.find((step) => step.status !== "done")?.title ?? <NotSet />}
          </Row>
        </>
      ) : (
        <p className="text-[12px] text-muted-foreground">No plans are registered.</p>
      )}
      {onOpen ? (
        <button
          className="inline-flex items-center gap-1 self-start text-[11px] text-primary hover:underline"
          onClick={onOpen}
          type="button"
        >
          Open plan <ArrowUpRightIcon className="size-3" />
        </button>
      ) : null}
    </Section>
  );
}

export function ActionsBlock({
  items,
  nextRunAt,
  onCancel,
}: {
  items: ActionStatusView[];
  nextRunAt: number | null;
  onCancel?: (actionId: string) => void;
}) {
  const now = useNow();
  return (
    <Section id="actions" title="Actions" count={items.length} updatedAt={nextRunAt ?? undefined}>
      <Row label="next due">{nextRunAt ? until(nextRunAt, now) : <NotSet>none scheduled</NotSet>}</Row>
      {items.length === 0 ? (
        <p className="text-[12px] text-muted-foreground">No scheduled or recent actions.</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {items.slice(0, 5).map((action) => (
            <li className="flex min-w-0 items-center gap-1.5 text-[12px]" key={action.actionId}>
              <span className="min-w-0 flex-1 truncate">{action.kind}</span>
              <Chip tone={action.status === "running" ? "live" : "neutral"}>{action.status}</Chip>
              <span className="font-mono text-[10px] text-muted-foreground">
                {action.dueAt ? until(action.dueAt, now) : `${action.runs} runs`}
              </span>
              {onCancel && action.status !== "cancelled" && action.status !== "done" ? (
                <button
                  aria-label={`Cancel ${action.kind}`}
                  className="text-muted-foreground hover:text-destructive"
                  onClick={() => onCancel(action.actionId)}
                  type="button"
                >
                  <XIcon className="size-3" />
                </button>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

export function AutomationBlock({ view }: { view: StatusView }) {
  const automation = view.automation;
  return (
    <Section id="automation" title="Automation" count={automation.supervisor}>
      <Row label="supervisor" mono>
        {automation.supervisor}
      </Row>
      <Row label="tick">{automation.tickSecs}s</Row>
      <Row label="sessions">{automation.sessions}</Row>
      <Row label="cron jobs">{automation.cronJobs}</Row>
      <Row label="frame timers">{automation.frameTimers}</Row>
      <Row label="next">{automation.nextActionAt ? until(automation.nextActionAt) : <NotSet />}</Row>
      <Row label="last">{automation.lastReason ?? <NotSet>none</NotSet>}</Row>
      {view.unavailable.automation ? (
        <UnavailableRow section="automation" reason={view.unavailable.automation} />
      ) : null}
    </Section>
  );
}

export function InterjectionBlock({ view }: { view: StatusView }) {
  const policy = view.interjections;
  return (
    <Section id="interjections" title="Interjections" count={policy.queue}>
      <Row label="policy" mono>
        {policy.policyId}
      </Row>
      <Row label="decision">
        <Chip tone={policy.suppressed ? "warn" : "ok"}>
          {policy.suppressed ? "suppressed" : "deliver"}
        </Chip>
        {policy.reason ? <span className="ml-1 text-muted-foreground">{policy.reason}</span> : null}
      </Row>
      <Row label="queue">{policy.queue}</Row>
    </Section>
  );
}

export function MemoryPanel({ view }: { view: StatusView }) {
  const reason = view.unavailable.memory;
  return (
    <Section defaultOpen={false} id="memory" title="Memory">
      <UnavailableRow
        section="memory"
        reason={reason ?? "this status view does not expose memory rows"}
      />
    </Section>
  );
}

function formatValue(value: StatusValue): string {
  if (value === null || value === undefined) return "not set";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) return value.map(formatValue).join(" · ");
  const record = asRecord(value);
  if (record) {
    const id = asString(record.id);
    const kind = asString(record.kind);
    if (id) return kind ? `${kind} · ${id}` : id;
  }
  return JSON.stringify(value);
}

const reservedStatus = new Set([
  "tasks",
  "next",
  "blockers",
  "questions",
  "notes",
  "focus",
  "phase",
  "intent",
  "frame_step",
  "frame_attempt",
  "awaiting_user",
  "next_utterance_at",
]);

export function StatusVariables({ view }: { view: StatusView }) {
  const status = view.state.status;
  const tasks = asList(status.tasks);
  const next = asList(status.next);
  const blockers = asList(status.blockers);
  const questions = asList(status.questions);
  const notes = asString(status.notes);
  const focus = status.focus;
  const custom = Object.entries(status).filter(([key]) => !reservedStatus.has(key));
  return (
    <Section id="status-variables" title="Tasks & next" count={tasks.length + next.length}>
      <Row label="tasks">
        {tasks.length ? tasks.join(" · ") : <NotSet>not set</NotSet>}
      </Row>
      <Row label="next">
        {next.length ? next.join(" · ") : <NotSet>not set</NotSet>}
      </Row>
      <Row label="blockers">
        {blockers.length ? blockers.join(" · ") : <NotSet>none</NotSet>}
      </Row>
      <Row label="questions">
        {questions.length ? questions.join(" · ") : <NotSet>none</NotSet>}
      </Row>
      <Row label="notes">{notes ?? <NotSet>not set</NotSet>}</Row>
      <Row label="focus" title={focus === undefined ? undefined : formatValue(focus)}>
        {focus === undefined ? <NotSet>not set</NotSet> : formatValue(focus)}
      </Row>
      {custom.length ? (
        <div className="mt-1 flex flex-col gap-0.5">
          <span className="text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
            custom variables
          </span>
          {custom.map(([key, value]) => (
            <Row key={key} label={key} title={formatValue(value)}>
              {formatValue(value)}
            </Row>
          ))}
        </div>
      ) : null}
    </Section>
  );
}

export function AgentRuns({
  agents,
  onOpen,
}: {
  agents: AgentStatusView[];
  onOpen?: (runId: string) => void;
}) {
  return (
    <Section defaultOpen={agents.length > 0} id="agents" title="Agents" count={agents.length}>
      {agents.length === 0 ? (
        <p className="text-[12px] text-muted-foreground">No agent runs for this session.</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {agents.map((run) => (
            <li className="flex min-w-0 items-center gap-1.5" key={run.runId}>
              <button
                className="flex min-w-0 flex-1 items-center gap-1.5 text-left hover:text-primary"
                onClick={() => onOpen?.(run.runId)}
                type="button"
              >
                <span className="min-w-0 flex-1 truncate text-[12px]">{run.kind}</span>
                <span className="font-mono text-[10px] text-muted-foreground">
                  {shortId(run.runId)}
                </span>
              </button>
              <Chip tone={run.active ? "live" : "neutral"}>{run.status}</Chip>
              <span className="font-mono text-[10px] text-muted-foreground">{ago(run.updatedAt)}</span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

function operationSummary(detail: StatusValue): string {
  if (typeof detail === "string") return detail;
  const record = asRecord(detail);
  if (!record) return JSON.stringify(detail);
  for (const key of ["summary", "message", "text", "tool", "title", "value"]) {
    const value = asString(record[key]);
    if (value) return value;
  }
  return JSON.stringify(detail).slice(0, 140);
}

export function OperationList({ view }: { view: StatusView }) {
  return (
    <Section id="operations" title="Recent operations" count={view.operations.length}>
      {view.operations.length === 0 ? (
        <p className="text-[12px] text-muted-foreground">No operations recorded.</p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {view.operations.slice(0, 8).map((operation) => (
            <li className="flex min-w-0 items-baseline gap-1.5 text-[12px]" key={operation.id}>
              <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                {clockAt(operation.at)}
              </span>
              <span className="shrink-0 font-mono text-[10px]">{operation.kind}</span>
              <span className="min-w-0 flex-1 truncate text-muted-foreground" title={operationSummary(operation.detail)}>
                {operationSummary(operation.detail)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

export function VariablesBlock({ view }: { view: StatusView }) {
  const budget = view.budget;
  return (
    <Section defaultOpen={false} id="variables" title="Internal variables">
      <Row label="state rev" mono>
        {view.stateRevision}
      </Row>
      <Row label="view rev" mono>
        {view.revision}
      </Row>
      <Row label="data rev" mono>
        {view.dataRevision}
      </Row>
      <Row label="depth">{view.state.depth}</Row>
      <Row label="agents">{view.agents.length}</Row>
      <Row label="operations">{view.operations.length}</Row>
      <Row label="budget class" mono>
        {budget.class}
      </Row>
      <Row label="tokens" mono>
        {compactTokens(budget.tokensUsed)} / {budget.tokenCap ? compactTokens(budget.tokenCap) : "not set"}
      </Row>
      <Row label="turns" mono>
        {budget.turnsUsed} / {budget.turnCap || "not set"}
      </Row>
      <Row label="index" mono>
        {view.data.indexPending} pending
      </Row>
    </Section>
  );
}

