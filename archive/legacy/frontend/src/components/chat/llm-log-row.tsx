import { Fragment } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";

import type { CallGroup, RequestRow } from "@/lib/store/llm-log.ts";
import {
  clock,
  detailRows,
  KIND_CLASS,
  KIND_LABEL,
  outcomeBody,
  outcomeMeta,
  outcomeSummary,
  outcomeTools,
  requestMeta,
  requestSummary,
  type OutcomeRow,
} from "./llm-log-format.ts";

function RowButton({
  at,
  kind,
  glyph,
  summary,
  meta,
  detail,
  open,
  onClick,
}: {
  at: number;
  kind: "request" | "response" | "error";
  glyph?: string;
  summary: string;
  meta: string;
  /** A third line that must stay visible without expanding the row. */
  detail?: string;
  open: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-expanded={open}
      onClick={onClick}
      className="hover:bg-muted/50 flex w-full min-w-0 items-baseline gap-2 px-2 py-1 text-left"
    >
      <span className="text-muted-foreground shrink-0 tabular-nums">{clock(at)}</span>
      <span aria-hidden className="text-muted-foreground w-2 shrink-0">
        {glyph ?? ""}
      </span>
      <span className={`w-7 shrink-0 font-semibold ${KIND_CLASS[kind]}`}>{KIND_LABEL[kind]}</span>
      <span className="min-w-0 flex-1">
        <span className="block truncate" title={summary}>
          {summary}
        </span>
        <span className="text-muted-foreground block truncate" title={meta}>
          {meta}
        </span>
        {detail ? (
          <span className="text-azure-600 block truncate" title={detail}>
            {detail}
          </span>
        ) : null}
      </span>
      <span className="text-muted-foreground shrink-0">
        {open ? <ChevronUp className="size-3" aria-hidden /> : <ChevronDown className="size-3" aria-hidden />}
      </span>
    </button>
  );
}

function RowDetails({ rows }: { rows: Array<[string, string]> }) {
  return (
    <dl className="text-muted-foreground grid grid-cols-[6rem_1fr] gap-x-2 gap-y-0.5 px-2 pb-2 pl-11">
      {rows.map(([label, value]) => (
        <Fragment key={label}>
          <dt className="shrink-0">{label}</dt>
          <dd className="text-foreground min-w-0 break-words whitespace-pre-wrap">{value}</dd>
        </Fragment>
      ))}
    </dl>
  );
}

function OutcomeRow({
  outcome,
  request,
  level,
  expanded,
  onToggle,
}: {
  outcome: OutcomeRow;
  request: RequestRow;
  level: number;
  expanded: ReadonlySet<string>;
  onToggle: (key: string) => void;
}) {
  const key = `${outcome.callId}:outcome`;
  const open = expanded.has(key);
  const body = outcomeBody(outcome, request);
  const summary = outcomeSummary(outcome, request);
  const tools = outcomeTools(outcome);
  // A tool-carried reply already *is* the preview; do not print it twice.
  const detail = tools && tools !== summary ? tools : undefined;

  return (
    <li
      role="treeitem"
      aria-level={level}
      data-kind={outcome.kind}
      data-llm-log-row
      data-call-id={outcome.callId}
      className="border-border/60 border-b last:border-b-0"
    >
      <RowButton
        at={outcome.at}
        kind={outcome.kind}
        glyph="↳"
        summary={summary}
        meta={outcomeMeta(outcome)}
        detail={detail}
        open={open}
        onClick={() => onToggle(key)}
      />
      {open ? (
        <>
          <RowDetails rows={detailRows(outcome)} />
          {body ? (
            <pre className="text-foreground bg-muted/40 mx-2 mb-2 max-h-48 overflow-auto rounded-md p-2 font-mono text-[11px] break-words whitespace-pre-wrap">
              {body}
            </pre>
          ) : null}
        </>
      ) : null}
    </li>
  );
}

export function CallGroupRow({
  group,
  level,
  expanded,
  onToggle,
}: {
  group: CallGroup;
  level: number;
  expanded: ReadonlySet<string>;
  onToggle: (key: string) => void;
}) {
  const key = `${group.callId}:request`;
  const open = expanded.has(key);

  return (
    <li
      role="treeitem"
      aria-level={level}
      aria-expanded={open}
      data-kind="request"
      data-llm-log-row
      data-call-id={group.callId}
      data-orphan={group.orphan ? "true" : undefined}
      className="border-border/60 border-b last:border-b-0"
    >
      <RowButton
        at={group.request.at}
        kind="request"
        glyph={level > 1 ? "↳" : undefined}
        summary={requestSummary(group.request)}
        meta={`${group.orphan ? "orphan · " : ""}${requestMeta(group.request)}`}
        open={open}
        onClick={() => onToggle(key)}
      />
      {open ? <RowDetails rows={detailRows(group.request, group)} /> : null}
      {group.outcome || group.children.length > 0 ? (
        <ul role="group" className="border-border/70 ml-3 border-l pl-2">
          {group.outcome ? (
            <OutcomeRow
              outcome={group.outcome}
              request={group.request}
              level={level + 1}
              expanded={expanded}
              onToggle={onToggle}
            />
          ) : null}
          {group.children.map((child) => (
            <CallGroupRow
              key={child.callId}
              group={child}
              level={Math.min(2, level + 1)}
              expanded={expanded}
              onToggle={onToggle}
            />
          ))}
        </ul>
      ) : null}
    </li>
  );
}
