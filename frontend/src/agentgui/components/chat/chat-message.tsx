"use client";

import {
  Confirmation,
  ConfirmationAccepted,
  ConfirmationAction,
  ConfirmationActions,
  ConfirmationRejected,
  ConfirmationRequest,
  ConfirmationTitle,
} from "@/agentgui/components/ai-elements/confirmation";
import {
  InlineCitation,
  InlineCitationCard,
  InlineCitationCardBody,
  InlineCitationCardTrigger,
  InlineCitationSource,
} from "@/agentgui/components/ai-elements/inline-citation";
import {
  MessageAction,
  MessageActions,
  MessageResponse,
} from "@/agentgui/components/ai-elements/message";
import {
  Plan,
  PlanContent,
  PlanDescription,
  PlanHeader,
  PlanTitle,
  PlanTrigger,
} from "@/agentgui/components/ai-elements/plan";
import { Questionnaire } from "@/agentgui/components/ai-elements/questionnaire";
import {
  Reasoning,
  ReasoningContent,
  ReasoningTrigger,
} from "@/agentgui/components/ai-elements/reasoning";
import {
  Source,
  Sources,
  SourcesContent,
  SourcesTrigger,
} from "@/agentgui/components/ai-elements/sources";
import { Subagent } from "@/agentgui/components/ai-elements/subagent";
import { Trace } from "@/agentgui/components/ai-elements/trace";
import { PassBlock } from "@/agentgui/components/chat/pass-block";
import { Badge } from "@/agentgui/components/ui/badge";
import { Button } from "@/agentgui/components/ui/button";
import { Textarea } from "@/agentgui/components/ui/textarea";
import { cn, formatTimestamp } from "@/agentgui/lib/utils";
import type { ChatMessage, MessagePart } from "@/agentgui/types/workspace";
import {
  CheckIcon,
  CircleAlertIcon,
  CircleCheckIcon,
  CopyIcon,
  LoaderCircleIcon,
  PencilIcon,
  RefreshCwIcon,
  SparklesIcon,
  XIcon,
} from "lucide-react";
import { useState } from "react";

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  return (
    <MessageAction
      onClick={() => {
        void navigator.clipboard?.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1_600);
      }}
      tooltip="Copy message"
    >
      {copied ? <CheckIcon className="size-3.5" /> : <CopyIcon className="size-3.5" />}
    </MessageAction>
  );
}

function CitationStrip({
  sources,
}: {
  sources: Extract<MessagePart, { type: "sources" }>;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1">
      {sources.sources.map((source) => (
        <InlineCitation key={source.index}>
          <InlineCitationCard>
            <InlineCitationCardTrigger sources={[source.url]} />
            <InlineCitationCardBody>
              <div className="space-y-2 p-3">
                <InlineCitationSource
                  description={source.snippet}
                  title={source.title}
                  url={source.url}
                />
              </div>
            </InlineCitationCardBody>
          </InlineCitationCard>
        </InlineCitation>
      ))}
    </div>
  );
}

function PlanBlock({
  plan,
}: {
  plan: Extract<MessagePart, { type: "plan" }>["plan"];
}) {
  return (
    <Plan defaultOpen>
      <PlanHeader>
        <div className="min-w-0">
          <PlanTitle>{plan.title}</PlanTitle>
          {plan.description && (
            <PlanDescription>{plan.description}</PlanDescription>
          )}
        </div>
        <PlanTrigger />
      </PlanHeader>
      <PlanContent className="space-y-1.5 pt-0">
        {plan.steps.map((step, index) => (
          <div className="flex items-center gap-2" key={step.id}>
            <span
              className={cn(
                "flex size-4 shrink-0 items-center justify-center rounded-full border text-[9px] font-medium",
                step.status === "done" &&
                  "border-positive/30 bg-positive/12 text-positive",
                step.status === "running" &&
                  "border-primary/30 bg-primary/10 text-primary",
                step.status === "pending" &&
                  "border-border bg-surface-muted text-muted-foreground"
              )}
            >
              {step.status === "done" ? (
                <CheckIcon className="size-2.5" />
              ) : (
                index + 1
              )}
            </span>
            <span
              className={cn(
                "text-[12.5px]",
                step.status === "done" && "text-muted-foreground line-through",
                step.status === "running" && "font-medium text-foreground",
                step.status === "pending" && "text-foreground/75"
              )}
            >
              {step.label}
            </span>
            {step.status === "running" && (
              <span className="ml-auto font-mono text-[10px] text-primary">
                in progress
              </span>
            )}
          </div>
        ))}
      </PlanContent>
    </Plan>
  );
}

function ApprovalBlock({
  payload,
}: {
  payload: Extract<MessagePart, { type: "confirmation" }>["payload"];
}) {
  const [decision, setDecision] = useState<boolean | undefined>(undefined);

  return (
    <Confirmation
      approval={
        decision === undefined
          ? { id: payload.id }
          : { id: payload.id, approved: decision }
      }
      className="border-caution/35 bg-caution/8"
      state={decision === undefined ? "approval-requested" : "approval-responded"}
    >
      <ConfirmationTitle className="text-[13px] font-medium">
        {payload.action}
      </ConfirmationTitle>
      <ConfirmationRequest>
        <p className="text-[12px] text-muted-foreground">{payload.detail}</p>
        {payload.toolName && (
          <Badge
            className="mt-1.5 font-mono text-[10px] font-normal"
            variant="secondary"
          >
            {payload.toolName}
          </Badge>
        )}
      </ConfirmationRequest>
      <ConfirmationAccepted>
        <p className="text-[12px] text-positive">
          Approved — the agent will continue with this step.
        </p>
      </ConfirmationAccepted>
      <ConfirmationRejected>
        <p className="text-[12px] text-muted-foreground">
          Rejected — the agent will look for another approach.
        </p>
      </ConfirmationRejected>
      <ConfirmationActions>
        <ConfirmationAction
          onClick={() => setDecision(false)}
          variant="ghost"
        >
          <XIcon className="size-3.5" /> Reject
        </ConfirmationAction>
        <ConfirmationAction onClick={() => setDecision(true)}>
          <CheckIcon className="size-3.5" /> Approve
        </ConfirmationAction>
      </ConfirmationActions>
    </Confirmation>
  );
}

function PartView({
  part,
  streaming,
}: {
  part: MessagePart;
  streaming: boolean;
}) {
  switch (part.type) {
    case "text":
      return (
        <MessageResponse className="markdown-body">{part.text}</MessageResponse>
      );

    case "thinking":
      return (
        <Reasoning
          className="rounded-xl border border-border/70 bg-surface-muted/50 px-3 py-2"
          isStreaming={streaming && !part.done}
        >
          <ReasoningTrigger className="text-[12px] [&_svg]:size-3.5" />
          <ReasoningContent className="mt-3 text-[12px] leading-relaxed">
            {part.text}
          </ReasoningContent>
        </Reasoning>
      );

    case "trace":
      return <Trace steps={part.steps} />;

    case "pass":
      return <PassBlock part={part} />;

    case "plan":
      return <PlanBlock plan={part.plan} />;

    case "sources":
      return (
        <div className="space-y-2">
          <Sources>
            <SourcesTrigger count={part.sources.length} />
            <SourcesContent>
              {part.sources.map((source) => (
                <Source
                  href={source.url}
                  key={source.index}
                  title={`${source.index}. ${source.title}`}
                />
              ))}
            </SourcesContent>
          </Sources>
          <CitationStrip sources={part} />
        </div>
      );

    case "questionnaire":
      return <Questionnaire payload={part.payload} />;

    case "confirmation":
      return <ApprovalBlock payload={part.payload} />;

    case "subagent":
      return (
        <Subagent
          model={part.model}
          name={part.name}
          status={part.status}
          steps={part.steps}
        />
      );

    default:
      return null;
  }
}

export function ChatMessageItem({
  message,
  canRetry,
  onRetry,
  onEdit,
}: {
  message: ChatMessage;
  canRetry?: boolean;
  onRetry?: () => void;
  onEdit?: (text: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");

  if (message.role === "user") {
    const text = message.parts.find((part) => part.type === "text")?.text ?? "";
    return (
      <div className="group flex flex-col items-end gap-1">
        {editing ? (
          <div className="w-full max-w-[92%] rounded-xl border border-primary/25 bg-surface p-2 shadow-elev-2">
            <Textarea
              autoFocus
              className="min-h-20 resize-none border-0 bg-transparent p-1 text-[13px] shadow-none focus-visible:ring-0"
              onChange={(event) => setDraft(event.target.value)}
              value={draft}
            />
            <div className="flex justify-end gap-1.5 pt-1">
              <Button
                onClick={() => setEditing(false)}
                size="sm"
                variant="ghost"
              >
                Cancel
              </Button>
              <Button
                onClick={() => {
                  onEdit?.(draft);
                  setEditing(false);
                }}
                size="sm"
              >
                Resend
              </Button>
            </div>
          </div>
        ) : (
          <>
            <div className="max-w-[92%] rounded-2xl rounded-br-md border border-primary/18 bg-gradient-to-br from-[oklch(0.985_0.006_255)] to-[oklch(0.972_0.012_245)] px-3.5 py-2.5 text-[13px] leading-relaxed shadow-elev-1">
              <p className="whitespace-pre-wrap">{text}</p>
            </div>
            <div className="flex items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
              <span className="mr-1 font-mono text-[10px] text-muted-foreground/70">
                {formatTimestamp(message.createdAt)}
              </span>
              <MessageActions>
                <CopyButton text={text} />
                <MessageAction
                  onClick={() => {
                    setDraft(text);
                    setEditing(true);
                  }}
                  tooltip="Edit and resend"
                >
                  <PencilIcon className="size-3.5" />
                </MessageAction>
              </MessageActions>
            </div>
          </>
        )}
      </div>
    );
  }

  const plainText = message.parts
    .filter((part) => part.type === "text")
    .map((part) => part.text)
    .join("\n\n");

  return (
    <div className="group">
      <div className="max-w-[90%] min-w-0">
        <div className="mb-2 flex items-center gap-1.5">
          <span className="accent-gradient flex size-[18px] shrink-0 items-center justify-center rounded-md text-white shadow-elev-1">
            <SparklesIcon className="size-3" />
          </span>
          <span className="text-[12.5px] font-semibold tracking-tight">Agent</span>
          {message.model && (
            <Badge
              className="h-4 rounded-md px-1.5 font-mono text-[9.5px] font-normal text-muted-foreground"
              variant="secondary"
            >
              {message.model}
            </Badge>
          )}
          <span className="font-mono text-[10px] text-muted-foreground/70">
            {formatTimestamp(message.createdAt)}
          </span>
          {message.status === "streaming" && (
            <LoaderCircleIcon className="size-3 animate-spin text-primary" />
          )}
          {message.status === "error" && (
            <span className="flex items-center gap-1 text-[10px] text-destructive">
              <CircleAlertIcon className="size-3" /> failed
            </span>
          )}
          {message.status === "cancelled" && (
            <span className="text-[10px] text-muted-foreground">stopped</span>
          )}
        </div>

        <div className="space-y-3">
          {message.parts.map((part, index) => (
            <PartView
              key={`${message.id}-${part.type}-${index}`}
              part={part}
              streaming={message.status === "streaming"}
            />
          ))}

          {message.status === "streaming" &&
            message.parts.length === 0 && (
              <div className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
                <span className="status-dot status-dot-busy" />
                Preparing response…
              </div>
            )}

          {message.status === "error" && message.error && (
            <div className="rounded-lg border border-destructive/25 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">
              {message.error}
            </div>
          )}
        </div>

        <div className="mt-2 flex items-center gap-2">
          {message.status === "complete" && (
            <MessageActions className="opacity-0 transition-opacity group-hover:opacity-100">
              <CopyButton text={plainText} />
              {canRetry && (
                <MessageAction onClick={onRetry} tooltip="Regenerate">
                  <RefreshCwIcon className="size-3.5" />
                </MessageAction>
              )}
            </MessageActions>
          )}
          {message.status === "complete" && (
            <span className="flex items-center gap-1 text-[10px] text-muted-foreground/60">
              <CircleCheckIcon className="size-2.5 text-positive" />
              {message.usage?.total
                ? `${message.usage.total.toLocaleString()} tokens`
                : "complete"}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
