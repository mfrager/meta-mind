"use client";

import {
  PromptInput,
  PromptInputActionAddAttachments,
  PromptInputActionMenu,
  PromptInputActionMenuContent,
  PromptInputActionMenuTrigger,
  PromptInputAttachments,
  PromptInputAttachment,
  PromptInputBody,
  PromptInputButton,
  PromptInputFooter,
  PromptInputHeader,
  PromptInputSubmit,
  PromptInputTextarea,
  PromptInputTools,
} from "@/agentgui/components/ai-elements/prompt-input";
import {
  Queue,
  QueueItem,
  QueueItemAction,
  QueueItemActions,
  QueueItemContent,
  QueueItemIndicator,
  QueueList,
  QueueSection,
  QueueSectionContent,
  QueueSectionLabel,
  QueueSectionTrigger,
} from "@/agentgui/components/ai-elements/queue";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/agentgui/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/agentgui/components/ui/tooltip";
import { useServerHealth } from "@/agentgui/hooks/use-server-health";
import { useChatStore } from "@/agentgui/lib/store/chat-store";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn } from "@/agentgui/lib/utils";
import type { ReasoningEffort } from "@/agentgui/types/workspace";
import {
  BrainIcon,
  ChevronDownIcon,
  CpuIcon,
  ListChecksIcon,
  PaperclipIcon,
  SquareIcon,
  XIcon,
} from "lucide-react";
import { useState } from "react";

const models = ["gpt-4o-mini", "gpt-4o", "o4-mini", "gpt-4.1-mini"];
const EMPTY_QUEUE: string[] = [];

const efforts: { id: ReasoningEffort; label: string }[] = [
  { id: "low", label: "Low — fastest" },
  { id: "medium", label: "Medium — balanced" },
  { id: "high", label: "High — deepest" },
];

export function ChatComposer({ sessionId }: { sessionId: string }) {
  const streaming = useChatStore(
    (state) => state.streamingSessions[sessionId] ?? false
  );
  const send = useChatStore((state) => state.send);
  const stop = useChatStore((state) => state.stop);
  const removeQueued = useChatStore((state) => state.removeQueued);
  const queue = useChatStore((state) => state.queue[sessionId]) ?? EMPTY_QUEUE;

  const model = useWorkspaceStore((state) => state.model);
  const setModel = useWorkspaceStore((state) => state.setModel);
  const reasoningEffort = useWorkspaceStore((state) => state.reasoningEffort);
  const setReasoningEffort = useWorkspaceStore(
    (state) => state.setReasoningEffort
  );
  const transport = useWorkspaceStore((state) => state.transport);
  const { health } = useServerHealth();
  const serverModel = health?.model ?? model;

  const [hasText, setHasText] = useState(false);

  return (
    <div className="shrink-0 px-3 pb-3" id="chat-composer">
      {queue.length > 0 && (
        <Queue className="mb-2 bg-surface-muted/50 py-1.5">
          <QueueSection>
            <QueueSectionTrigger>
              <QueueSectionLabel
                count={queue.length}
                icon={<ListChecksIcon className="size-3.5" />}
                label="queued"
              />
            </QueueSectionTrigger>
            <QueueSectionContent>
              <QueueList>
                {queue.map((prompt, index) => (
                  <QueueItem key={`${prompt}-${index}`}>
                    <div className="flex items-start gap-2">
                      <QueueItemIndicator />
                      <QueueItemContent className="text-[12px]">
                        {prompt}
                      </QueueItemContent>
                      <QueueItemActions className="ml-auto">
                        <QueueItemAction
                          aria-label="Remove queued prompt"
                          onClick={() => removeQueued(sessionId, index)}
                        >
                          <XIcon className="size-3" />
                        </QueueItemAction>
                      </QueueItemActions>
                    </div>
                  </QueueItem>
                ))}
              </QueueList>
            </QueueSectionContent>
          </QueueSection>
        </Queue>
      )}

      <PromptInput
        className={cn(
          "rounded-lg",
          "[&_[data-slot=input-group]]:rounded-lg [&_[data-slot=input-group]]:border-border [&_[data-slot=input-group]]:bg-surface [&_[data-slot=input-group]]:shadow-elev-2"
        )}
        multiple
        onSubmit={(message) => {
          if (!message.text.trim()) return;
          send(sessionId, message.text);
          setHasText(false);
        }}
      >
        <PromptInputHeader className="px-3 pt-2.5 pb-0">
          <PromptInputAttachments>
            {(file) => <PromptInputAttachment data={file} />}
          </PromptInputAttachments>
        </PromptInputHeader>

        <PromptInputBody>
          <PromptInputTextarea
            className="min-h-[56px] px-3.5 pt-2.5 text-[13px] leading-relaxed"
            onChange={(event) => setHasText(event.target.value.length > 0)}
            placeholder="Ask the agent to inspect, build or explain something…"
          />
        </PromptInputBody>

        <PromptInputFooter className="px-2 pb-2">
          <PromptInputTools>
            <PromptInputActionMenu>
              <Tooltip>
                <TooltipTrigger asChild>
                  <PromptInputActionMenuTrigger aria-label="Attach files">
                    <PaperclipIcon className="size-3.5" />
                  </PromptInputActionMenuTrigger>
                </TooltipTrigger>
                <TooltipContent>Attach files</TooltipContent>
              </Tooltip>
              <PromptInputActionMenuContent>
                <PromptInputActionAddAttachments label="Upload files" />
              </PromptInputActionMenuContent>
            </PromptInputActionMenu>

            {transport === "server" ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="flex h-7 items-center gap-1 rounded-md px-2 text-[11.5px] text-muted-foreground">
                    <CpuIcon className="size-3.5" />
                    <span className="max-w-32 truncate font-mono">
                      {serverModel}
                    </span>
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  Set by OPENAI_MODEL on the server
                </TooltipContent>
              </Tooltip>
            ) : (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <PromptInputButton className="gap-1 px-2 text-[11.5px] text-muted-foreground">
                    <CpuIcon className="size-3.5" />
                    <span className="max-w-28 truncate">{model}</span>
                    <ChevronDownIcon className="size-3 opacity-60" />
                  </PromptInputButton>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="w-56">
                  <DropdownMenuLabel>Model</DropdownMenuLabel>
                  <DropdownMenuRadioGroup
                    onValueChange={setModel}
                    value={model}
                  >
                    {models.map((entry) => (
                      <DropdownMenuRadioItem key={entry} value={entry}>
                        <span className="font-mono text-[12px]">{entry}</span>
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                  <DropdownMenuSeparator />
                  <p className="px-2 py-1 text-[10px] text-muted-foreground">
                    Used by the local mock transport.
                  </p>
                </DropdownMenuContent>
              </DropdownMenu>
            )}

            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <PromptInputButton className="gap-1 px-2 text-[11.5px] text-muted-foreground capitalize">
                  <BrainIcon className="size-3.5" />
                  {reasoningEffort}
                  <ChevronDownIcon className="size-3 opacity-60" />
                </PromptInputButton>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-52">
                <DropdownMenuLabel>Reasoning effort</DropdownMenuLabel>
                <DropdownMenuRadioGroup
                  onValueChange={(value) =>
                    setReasoningEffort(value as ReasoningEffort)
                  }
                  value={reasoningEffort}
                >
                  {efforts.map((effort) => (
                    <DropdownMenuRadioItem key={effort.id} value={effort.id}>
                      {effort.label}
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          </PromptInputTools>

          <PromptInputTools>
            <span className="hidden font-mono text-[10px] text-muted-foreground/70 sm:block">
              ⏎ send · ⇧⏎ newline
            </span>
            {streaming ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <PromptInputButton
                    className="accent-gradient rounded-full text-white shadow-elev-2 hover:opacity-90"
                    onClick={() => stop(sessionId)}
                  >
                    <SquareIcon className="size-3 fill-current" />
                  </PromptInputButton>
                </TooltipTrigger>
                <TooltipContent>Stop generating</TooltipContent>
              </Tooltip>
            ) : (
              <PromptInputSubmit
                className={cn(
                  "rounded-full",
                  hasText
                    ? "accent-gradient text-white shadow-elev-2"
                    : "bg-surface-sunken text-muted-foreground"
                )}
                disabled={!hasText}
                status="ready"
              />
            )}
          </PromptInputTools>
        </PromptInputFooter>
      </PromptInput>

      <div className="mt-1.5 flex items-center justify-between px-1">
        <p className="font-mono text-[10px] text-muted-foreground/70">
          {transport === "server"
            ? "metamind · kernel SSE"
            : "local mock transport"}
        </p>
        <p className="text-[10px] text-muted-foreground/70">
          The agent shows observable execution, not private reasoning.
        </p>
      </div>
    </div>
  );
}
