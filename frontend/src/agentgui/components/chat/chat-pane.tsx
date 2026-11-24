"use client";

import { ChatComposer } from "@/agentgui/components/chat/chat-composer";
import { ChatMessageItem } from "@/agentgui/components/chat/chat-message";
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
} from "@/agentgui/components/ai-elements/message-scroller";
import { Badge } from "@/agentgui/components/ui/badge";
import { Button } from "@/agentgui/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/agentgui/components/ui/dropdown-menu";
import { useChatStore } from "@/agentgui/lib/store/chat-store";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn } from "@/agentgui/lib/utils";
import {
  ChartColumnIcon,
  Code2Icon,
  EllipsisIcon,
  FileTextIcon,
  PanelRightIcon,
  SearchIcon,
  SparklesIcon,
  Trash2Icon,
} from "lucide-react";
import type { ChatMessage } from "@/agentgui/types/workspace";
import type { LucideIcon } from "lucide-react";

const suggestions: { icon: LucideIcon; label: string; prompt: string }[] = [
  {
    icon: SearchIcon,
    label: "Summarize the kernel state",
    prompt:
      "Summarize what the metamind kernel is doing right now: services, budgets, goals and the last action. Flag anything degraded.",
  },
  {
    icon: Code2Icon,
    label: "Review recent memory",
    prompt:
      "Show me the most recent memory entries and tell me which ones look stale, contradictory or worth promoting to a goal.",
  },
  {
    icon: ChartColumnIcon,
    label: "Explain a signal",
    prompt:
      "Explain the most important signal in the current state board and what I should do about it next.",
  },
  {
    icon: FileTextIcon,
    label: "Draft a status memo",
    prompt:
      "Draft a short status memo covering what the kernel has done recently, open risks and next steps.",
  },
];

function EmptyState({ onPick }: { onPick: (prompt: string) => void }) {
  return (
    <div className="mx-auto flex w-full max-w-xl flex-col items-center justify-center gap-6 py-12 text-center">
      <div className="accent-gradient flex size-12 items-center justify-center rounded-2xl text-white shadow-elev-3">
        <SparklesIcon className="size-6" />
      </div>
      <div className="space-y-1.5">
        <h2 className="text-lg font-semibold tracking-tight">
          What should we work on?
        </h2>
        <p className="mx-auto max-w-sm text-[13px] leading-relaxed text-muted-foreground">
          Ask about the workspace, run an analysis, or have the agent produce an
          artifact in the panel beside us.
        </p>
      </div>

      <div className="grid w-full grid-cols-1 gap-1.5 sm:grid-cols-2">
        {suggestions.map((suggestion) => {
          const Icon = suggestion.icon;
          return (
            <button
              className="group flex items-start gap-2 rounded-xl border border-border bg-surface/70 px-3 py-2.5 text-left shadow-elev-1 transition-all hover:-translate-y-px hover:border-primary/25 hover:shadow-elev-2"
              key={suggestion.label}
              onClick={() => onPick(suggestion.prompt)}
              type="button"
            >
              <span className="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md bg-surface-sunken text-muted-foreground transition-colors group-hover:accent-soft group-hover:text-primary">
                <Icon className="size-3.5" />
              </span>
              <span className="text-[12.5px] font-medium leading-snug">
                {suggestion.label}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

const EMPTY_MESSAGES: ChatMessage[] = [];

export function ChatPane({ sessionId }: { sessionId: string }) {
  const messages =
    useChatStore((state) => state.messagesBySession[sessionId]) ??
    EMPTY_MESSAGES;
  const streaming = useChatStore(
    (state) => state.streamingSessions[sessionId] ?? false
  );
  const send = useChatStore((state) => state.send);
  const retry = useChatStore((state) => state.retry);
  const editAndResend = useChatStore((state) => state.editAndResend);
  const clearSession = useChatStore((state) => state.clearSession);

  const session = useWorkspaceStore((state) =>
    state.sessions.find((entry) => entry.id === sessionId)
  );
  const project = useWorkspaceStore((state) =>
    state.projects.find((entry) => entry.id === session?.projectId)
  );
  const paneLayout = useWorkspaceStore((state) => state.paneLayout);
  const setPaneLayout = useWorkspaceStore((state) => state.setPaneLayout);

  const lastAssistantIndex = messages.findLastIndex(
    (message) => message.role === "assistant"
  );

  return (
    <div className="flex h-full min-h-0 flex-col" data-agent-chat-pane>
      <div className="pane-header flex h-11 shrink-0 items-center gap-2 px-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <h1 className="truncate text-[14px] font-semibold tracking-tight">
              {session?.title ?? "New session"}
            </h1>
            {session?.status === "active" && (
              <span className="status-dot shrink-0" />
            )}
          </div>
          <p className="truncate text-[11.5px] text-muted-foreground">
            {project ? `${project.name} · ` : ""}
            {messages.length} messages
            {session?.tags?.length
              ? ` · ${session.tags.join(" · ")}`
              : ""}
          </p>
        </div>

        <Badge
          className={cn(
            "hidden h-5 rounded-md px-1.5 font-mono text-[11px] font-normal sm:flex",
            streaming ? "accent-soft text-primary" : "text-muted-foreground"
          )}
          variant="secondary"
        >
          {streaming ? "streaming" : "idle"}
        </Badge>

        <Button
          className="h-7 gap-1 px-2 text-[11px]"
          onClick={() =>
            setPaneLayout(paneLayout === "workspace" ? "two" : "workspace")
          }
          size="sm"
          variant="outline"
        >
          <PanelRightIcon className="size-3.5" />
          <span className="hidden sm:inline">Workspace</span>
        </Button>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Chat options"
              size="icon-sm"
              variant="ghost"
            >
              <EllipsisIcon className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48">
            <DropdownMenuItem onClick={() => clearSession(sessionId)}>
              <Trash2Icon className="size-3.5" /> Clear conversation
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onClick={() =>
                useWorkspaceStore.getState().setSidebarTab("sessions")
              }
            >
              <SearchIcon className="size-3.5" /> Browse sessions
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <MessageScroller className="bg-surface">
        <MessageScrollerContent className="mx-auto w-full max-w-3xl">
          {messages.length === 0 ? (
            <EmptyState onPick={(prompt) => send(sessionId, prompt)} />
          ) : (
            messages.map((message, index) => (
              <ChatMessageItem
                canRetry={
                  message.role === "assistant" &&
                  index === lastAssistantIndex &&
                  !streaming
                }
                key={message.id}
                message={message}
                onEdit={(text) => editAndResend(sessionId, message.id, text)}
                onRetry={() => retry(sessionId)}
              />
            ))
          )}
        </MessageScrollerContent>
        <MessageScrollerButton />
      </MessageScroller>

      <ChatComposer sessionId={sessionId} />
    </div>
  );
}
