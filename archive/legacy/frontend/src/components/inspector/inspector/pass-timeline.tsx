import { Chip } from "./section.tsx";
import { useMemo } from "react";

import type { AgentstreamEvent, StatusValue } from "@/lib/wire/inspector.ts";
import { asList, asString } from "@/lib/inspector/format.ts";

type PassRow = {
  key: string;
  pass: string;
  attempt: number;
  finished?: string;
  tokens?: number;
  promptHash?: string;
  checks: { label: string; pass: boolean; confidence: number }[];
};

function record(value: StatusValue): Record<string, StatusValue> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, StatusValue>)
    : {};
}

function frameOf(event: AgentstreamEvent): Record<string, StatusValue> {
  const payload = record(event.payload);
  return payload.frame ? record(payload.frame) : payload;
}

function numberAt(value: StatusValue, key: string): number | undefined {
  const found = record(value)[key];
  return typeof found === "number" ? found : undefined;
}

/**
 * Groups the pass events of a trace into one row per (turn, pass, attempt).
 *
 * Every field comes from an emitted event: a pass with no completion event is
 * shown as started but unfinished rather than guessed at.
 */
export function buildPassTimeline(events: AgentstreamEvent[]): PassRow[] {
  const rows: PassRow[] = [];
  const index = new Map<string, PassRow>();
  const rowFor = (event: AgentstreamEvent, pass: string, attempt: number) => {
    const key = `${event.turn_id ?? "unassigned"}:${pass}:${attempt}`;
    const existing = index.get(key);
    if (existing) return existing;
    const row: PassRow = { key, pass, attempt, checks: [] };
    index.set(key, row);
    rows.push(row);
    return row;
  };

  for (const event of events) {
    const frame = frameOf(event);
    if (event.event_type === "pass_started") {
      const pass = asString(frame.pass) ?? "pass";
      rowFor(event, pass, numberAt(frame, "attempt") ?? 1);
      continue;
    }
    if (event.event_type === "pass_completed") {
      const pass = asString(frame.pass) ?? "pass";
      const row = rowFor(event, pass, numberAt(frame, "attempt") ?? 1);
      row.finished = asString(frame.finished) ?? row.finished;
      row.tokens = numberAt(record(frame.budget), "tokens") ?? row.tokens;
      row.promptHash = asString(frame.promptHash) ?? row.promptHash;
      continue;
    }
    if (event.event_type === "verification_registered") {
      const payload = record(event.payload);
      for (const label of asList(payload.checks)) {
        const row = rowFor(event, "interpret", numberAt(payload, "attempt") ?? 1);
        if (typeof label === "string" && !row.checks.some((check) => check.label === label)) {
          row.checks.push({ label, pass: true, confidence: Number.NaN });
        }
      }
      continue;
    }
    if (event.event_type === "verification_result") {
      const label = asString(frame.label);
      if (!label) continue;
      const row = rowFor(event, "verify", 1);
      row.checks.push({
        label,
        pass: frame.pass === true,
        confidence: typeof frame.confidence === "number" ? frame.confidence : Number.NaN,
      });
    }
  }
  return rows;
}

function passTone(row: PassRow) {
  if (row.finished === "approved" || row.finished === "approved_override") return "ok" as const;
  if (row.finished === "rejected" || row.finished === "protocol_error" || row.finished === "error") {
    return "warn" as const;
  }
  if (row.finished === "exhausted") return "warn" as const;
  return "neutral" as const;
}

/** A compact pass timeline for the selected trace. */
export function PassTimeline({ events }: { events: AgentstreamEvent[] }) {
  const rows = useMemo(() => buildPassTimeline(events), [events]);
  if (rows.length === 0) return null;
  return (
    <div className="border-border/70 flex flex-col gap-1 border-b px-2 py-1.5" data-panel="pass-timeline">
      <span className="text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
        Passes
      </span>
      {rows.map((row) => (
        <div className="flex flex-wrap items-center gap-1 font-mono text-[10px]" key={row.key}>
          <Chip tone={passTone(row)}>
            {row.pass} · {row.attempt}
          </Chip>
          <span className="text-muted-foreground">{row.finished ?? "started"}</span>
          {row.tokens !== undefined ? <span className="text-muted-foreground">{row.tokens} tok left</span> : null}
          {row.promptHash ? (
            <span className="text-muted-foreground" title={row.promptHash}>
              #{row.promptHash.slice(0, 8)}
            </span>
          ) : null}
          {row.checks.map((check, position) => (
            <span
              className={check.pass ? "text-green-700 dark:text-green-400" : "text-amber-700 dark:text-amber-400"}
              key={`${check.label}:${position}`}
            >
              {check.label} {Number.isNaN(check.confidence) ? "registered" : `${check.pass ? "✓" : "✗"} ${check.confidence.toFixed(2)}`}
            </span>
          ))}
        </div>
      ))}
    </div>
  );
}
