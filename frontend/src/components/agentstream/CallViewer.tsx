import { useMemo, useState, type ReactNode } from "react";

import { Button } from "@/components/ui/actions.tsx";
import { Tabs } from "@/components/ui/navigation.tsx";
import { requestToolRecords } from "@/lib/store/agentstream.ts";
import {
  clock,
  dataOf,
  list,
  num,
  object,
  passOf,
  requestJson,
  str,
  usageOf,
} from "@/lib/transcript/model.ts";
import type { Event } from "@/lib/wire/agentstream.ts";

const TABS = [
  { id: "prompt", label: "Prompt" },
  { id: "tools", label: "Tools" },
  { id: "response", label: "Response" },
  { id: "internals", label: "Internals" },
];

/**
 * The expanded model call. Every field is read from `record.data`; nothing is
 * reconstructed or re-truncated here.
 */
export function CallViewer({ request, records }: { request: Event; records: Event[] }) {
  const [tab, setTab] = useState("prompt");
  const related = useMemo(() => relatedRecords(request, records), [request, records]);

  const copy = () => {
    void navigator.clipboard?.writeText(requestJson(request));
  };

  return (
    <section data-call-viewer={request.seq} className="border-border bg-background mt-1 rounded-md border">
      <div className="flex items-center justify-between gap-2 px-2 pt-1">
        <Tabs tabs={TABS} value={tab} onChange={setTab} label="Model call detail" />
        <Button type="button" variant="outline" onClick={copy}>
          Copy JSON
        </Button>
      </div>
      <div className="p-2 text-xs">
        {tab === "prompt" ? <Prompt messages={list(dataOf(request).messages)} /> : null}
        {tab === "tools" ? <Tools tools={list(dataOf(request).tools)} records={related.tools} /> : null}
        {tab === "response" ? <Response related={related} /> : null}
        {tab === "internals" ? <Internals request={request} related={related} /> : null}
      </div>
    </section>
  );
}

type Related = {
  response?: Event;
  error?: Event;
  attempt?: Event;
  prompt?: Event;
  tools: Event[];
  states: Event[];
  verifications: Event[];
  actions: Event[];
};

function relatedRecords(request: Event, records: Event[]): Related {
  const callId = str(dataOf(request).callId);
  const pass = passOf(request);
  const sameTurn = records.filter((event) => event.turnId === request.turnId);
  return {
    response: records.find((event) => event.type === "llm.response" && str(dataOf(event).callId) === callId),
    error: records.find((event) => event.type === "llm.error" && str(dataOf(event).callId) === callId),
    attempt: records.find((event) => event.type === "llm.attempt" && str(dataOf(event).callId) === callId),
    prompt: records.find((event) => event.type === "prompt.rendered" && passOf(event) === pass),
    tools: requestToolRecords(records, request),
    states: sameTurn.filter((event) => event.type === "state.updated"),
    verifications: sameTurn.filter((event) => event.type === "verification.result"),
    actions: sameTurn.filter((event) => event.type === "internal.action" && passOf(event) === pass),
  };
}

function Prompt({ messages }: { messages: unknown[] }) {
  if (messages.length === 0) return <p className="text-muted-foreground">No messages in this request.</p>;
  return (
    <ol data-viewer-prompt className="space-y-2">
      {messages.map((value, index) => {
        const message = object(value);
        const role = str(message.role) ?? "unknown";
        const content = str(message.content) ?? "";
        const calls = list(message.tool_calls);
        return (
          <li key={`${role}:${index}`} data-message-role={role} className="border-border rounded-xs border p-2">
            <p className="text-muted-foreground mb-1 flex flex-wrap gap-2 font-mono">
              <span>{index + 1}</span>
              <span className="text-foreground uppercase">{role}</span>
              {str(message.name) ? <span>name={str(message.name)}</span> : null}
              {str(message.tool_call_id) ? <span>tool_call_id={str(message.tool_call_id)}</span> : null}
            </p>
            {content ? <pre className="whitespace-pre-wrap break-words font-mono">{content}</pre> : null}
            {calls.map((value, callIndex) => {
              const call = object(value);
              return (
                <div key={callIndex} className="mt-1 border-l-2 border-azure-400 pl-2">
                  <p className="font-mono">tool call {str(call.name) ?? ""}</p>
                  <pre className="whitespace-pre-wrap break-words font-mono">{str(call.arguments) ?? ""}</pre>
                </div>
              );
            })}
          </li>
        );
      })}
    </ol>
  );
}

function Tools({ tools, records }: { tools: unknown[]; records: Event[] }) {
  const called = new Set(records.map((event) => str(dataOf(event).name)).filter(Boolean));
  return (
    <div data-viewer-tools className="space-y-2">
      <p className="text-muted-foreground">{tools.length} offered</p>
      <ul className="space-y-2">
        {tools.map((value, index) => {
          const tool = object(value);
          const name = str(tool.name) ?? `tool-${index}`;
          return (
            <li key={name} data-tool-name={name} className="border-border rounded-xs border p-2">
              <p className="font-mono">
                {name}
                {called.has(name) ? <span className="ml-2 text-emerald-700">called</span> : null}
              </p>
              <p className="text-muted-foreground">{str(tool.description) ?? ""}</p>
              <pre className="mt-1 overflow-auto whitespace-pre-wrap break-words font-mono">
                {JSON.stringify(tool.parameters, null, 2)}
              </pre>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function Response({ related }: { related: Related }) {
  if (related.error) {
    const error = dataOf(related.error);
    return (
      <div data-viewer-response className="space-y-1">
        <p className="text-red-700">{str(error.kind) ?? "error"}</p>
        <pre className="whitespace-pre-wrap break-words font-mono">{str(error.message) ?? ""}</pre>
      </div>
    );
  }
  if (!related.response) {
    return <p className="text-muted-foreground">No response yet. The live text is in the row above.</p>;
  }
  const response = dataOf(related.response);
  const usage = usageOf(related.response);
  const attempt = related.attempt ? dataOf(related.attempt) : {};
  const calls = list(response.toolCalls);
  return (
    <div data-viewer-response className="space-y-2">
      <p className="text-muted-foreground flex flex-wrap gap-2">
        <span>{str(response.model) ?? ""}</span>
        <span>finish={str(response.finishReason) ?? "unknown"}</span>
        {usage ? <span>tokens {usage.tokensIn}/{usage.tokensOut}</span> : null}
        <span>{num(response.latencyMs) ?? 0} ms</span>
        <span>attempt {num(attempt.attemptNo) ?? 1}</span>
        <span>{clock(related.response.at)}</span>
      </p>
      <pre className="whitespace-pre-wrap break-words font-mono">{str(response.text) ?? "(empty)"}</pre>
      {calls.length ? (
        <div>
          <p className="text-muted-foreground">Tool calls</p>
          {calls.map((value, index) => {
            const call = object(value);
            return (
              <p key={index} className="font-mono">
                {str(call.name) ?? ""} {str(call.arguments) ?? ""}
              </p>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

function Internals({ request, related }: { request: Event; related: Related }) {
  return (
    <div data-viewer-internals className="space-y-2">
      <Block title="prompt.rendered">
        {related.prompt ? (
          <>
            <p className="font-mono">
              {str(dataOf(related.prompt).strategy) ?? ""} · {str(dataOf(related.prompt).hash) ?? ""} ·{" "}
              {num(dataOf(related.prompt).messageCount) ?? 0} messages
            </p>
            <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words font-mono">
              {JSON.stringify(dataOf(related.prompt).variables, null, 2)}
            </pre>
          </>
        ) : (
          <p className="text-muted-foreground">No rendered-prompt record for this pass.</p>
        )}
      </Block>
      <Block title="state.updated">
        {related.states.length ? (
          related.states.map((state) => (
            <div key={state.seq} className="space-y-1">
              <p className="font-mono">revision {num(dataOf(state).revision) ?? 0}</p>
              <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono">
                before {JSON.stringify(dataOf(state).before, null, 2)}
              </pre>
              <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono">
                after {JSON.stringify(dataOf(state).after, null, 2)}
              </pre>
            </div>
          ))
        ) : (
          <p className="text-muted-foreground">No state commit in this turn.</p>
        )}
      </Block>
      <Block title="verification.result">
        {related.verifications.length ? (
          related.verifications.map((event) => {
            const data = dataOf(event);
            return (
              <p key={event.seq} className="font-mono">
                {str(data.label) ?? ""} · {data.pass === true ? "pass" : "fail"} · {str(data.backend) ?? ""} ·{" "}
                {str(data.reasoning) ?? ""}
              </p>
            );
          })
        ) : (
          <p className="text-muted-foreground">No verification result in this turn.</p>
        )}
      </Block>
      <Block title="internal.action">
        {related.actions.length ? (
          related.actions.map((event) => {
            const data = dataOf(event);
            return (
              <p key={event.seq} className="font-mono">
                {str(data.action) ?? ""} · {data.ok === false ? "failed" : "ok"} · {num(data.durationMs) ?? 0} ms
              </p>
            );
          })
        ) : (
          <p className="text-muted-foreground">No internal action for this pass.</p>
        )}
      </Block>
      <Block title="request" collapsed>
        <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words font-mono">
          {requestJson(request)}
        </pre>
      </Block>
    </div>
  );
}

function Block({ title, children, collapsed = false }: { title: string; children: ReactNode; collapsed?: boolean }) {
  return (
    <details open={!collapsed} className="border-border rounded-xs border p-2">
      <summary className="font-mono">{title}</summary>
      <div className="mt-1 space-y-1">{children}</div>
    </details>
  );
}
