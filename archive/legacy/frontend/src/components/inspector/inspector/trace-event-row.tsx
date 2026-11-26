import type { AgentstreamEvent, StatusValue } from "@/lib/wire/inspector.ts";
import { asList, asString, clockAt } from "@/lib/inspector/format.ts";
import { eventRole, eventText } from "@/lib/api/agentstream.ts";
import { cn } from "@/lib/utils";

function record(value: StatusValue): Record<string, StatusValue> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, StatusValue>)
    : {};
}

function pretty(value: StatusValue): string {
  if (typeof value === "string") {
    try {
      return JSON.stringify(JSON.parse(value), null, 2);
    } catch {
      return value;
    }
  }
  return JSON.stringify(value, null, 2);
}

function toolName(payload: Record<string, StatusValue>): string {
  return asString(payload.tool) ?? asString(payload.name) ?? asString(payload.tool_name) ?? "tool";
}

function toolArguments(event: AgentstreamEvent): StatusValue | undefined {
  const payload = record(event.payload);
  return payload.arguments ?? payload.args ?? payload.input;
}

function toolResult(event: AgentstreamEvent): StatusValue | undefined {
  const payload = record(event.payload);
  return payload.result ?? payload.output ?? payload.content ?? payload.message;
}

function toolOk(event: AgentstreamEvent): boolean | null {
  const payload = record(event.payload);
  const ok = payload.ok;
  if (typeof ok === "boolean") return ok;
  const status = asString(payload.status);
  if (status) return !["failed", "error", "denied"].includes(status);
  return null;
}

function duration(event: AgentstreamEvent): number | null {
  const value = record(event.payload).durationMs ?? record(event.payload).duration_ms;
  return typeof value === "number" ? value : null;
}

function SummaryBody({ event }: { event: AgentstreamEvent }) {
  const payload = record(event.payload);
  const summary = record(payload.summary);
  const headline =
    asString(summary.headline) ?? asString(payload.summary) ?? asString(payload.headline) ?? eventText(event);
  const did = asList(summary.did);
  const learned = asList(summary.learned);
  const blockers = asList(summary.blockers);
  const next = asString(summary.nextRecommended) ?? asString(summary.next_recommended);
  return (
    <div className="flex flex-col gap-0.5">
      <p className="text-[12px] font-medium">{headline || "summary has no headline"}</p>
      {did.length ? <p className="text-[11px] text-muted-foreground">did: {did.join(" · ")}</p> : null}
      {learned.length ? <p className="text-[11px] text-muted-foreground">learned: {learned.join(" · ")}</p> : null}
      {blockers.length ? (
        <p className="text-[11px] text-amber-700 dark:text-amber-300">blockers: {blockers.join(" · ")}</p>
      ) : null}
      {next ? <p className="text-[11px] text-muted-foreground">next: {next}</p> : null}
    </div>
  );
}

export function ToolCard({ event }: { event: AgentstreamEvent }) {
  const payload = record(event.payload);
  const name = toolName(payload);
  const call = event.event_type === "tool_call";
  const ok = toolOk(event);
  const ms = duration(event);
  const body = call ? toolArguments(event) : toolResult(event);
  return (
    <div className="mt-1 rounded-sm border border-border bg-muted/25 p-1.5" data-tool={name}>
      <div className="flex items-center gap-1.5">
        <span className="min-w-0 flex-1 truncate font-mono text-[11px]">{name}</span>
        {call ? <span className="text-[10px] text-muted-foreground">call</span> : null}
        {ok !== null ? (
          <span className={cn("text-[10px] font-medium", ok ? "text-emerald-700 dark:text-emerald-300" : "text-red-700 dark:text-red-300")}>
            {ok ? "ok" : "fail"}
          </span>
        ) : null}
        {ms !== null ? <span className="font-mono text-[10px] text-muted-foreground">{ms}ms</span> : null}
      </div>
      {body !== undefined ? (
        <details className="mt-1">
          <summary className="cursor-pointer text-[10px] text-muted-foreground">
            {call ? "arguments" : "result"}
          </summary>
          <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap rounded-sm bg-background/70 p-1.5 font-mono text-[10px]">
            {pretty(body).slice(0, 8_000)}
          </pre>
        </details>
      ) : null}
    </div>
  );
}

const roleTone: Record<string, string> = {
  user: "border-l-sky-500",
  agent: "border-l-primary",
  tool: "border-l-violet-500",
  summary: "border-l-emerald-500",
  status: "border-l-amber-500",
  system: "border-l-muted-foreground/40",
};

export function TraceEventRow({
  event,
  raw,
  showRole = true,
}: {
  event: AgentstreamEvent;
  raw: boolean;
  showRole?: boolean;
}) {
  const role = eventRole(event);
  const isTool = event.event_type === "tool_call" || event.event_type === "tool_result";
  const isSummary = event.event_type === "agent_turn_summary";
  return (
    <article
      className={cn(
        "border-border/50 flex gap-2 border-b border-l-2 px-2 py-1.5",
        roleTone[role] ?? roleTone.system,
      )}
      data-event-id={event.event_id}
      data-event-type={event.event_type}
      data-role={role}
    >
      <span className="w-14 shrink-0 pt-px font-mono text-[9px] text-muted-foreground">
        {clockAt(event.timestamp)}
      </span>
      <div className="min-w-0 flex-1">
        {showRole ? (
          <span className="mb-0.5 block text-[9px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
            {role} · {event.event_type}
          </span>
        ) : null}
        {isTool ? (
          <ToolCard event={event} />
        ) : isSummary && !raw ? (
          <SummaryBody event={event} />
        ) : (
          <p className="break-words whitespace-pre-wrap text-[12px] leading-5">
            {raw ? pretty(event.payload) : eventText(event) || <span className="text-muted-foreground italic">empty payload</span>}
          </p>
        )}
      </div>
    </article>
  );
}
