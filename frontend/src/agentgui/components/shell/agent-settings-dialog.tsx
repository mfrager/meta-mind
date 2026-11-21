"use client";

import { Button } from "@/agentgui/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/agentgui/components/ui/dialog";
import { Input } from "@/agentgui/components/ui/input";
import { Textarea } from "@/agentgui/components/ui/textarea";
import { useServerHealth } from "@/agentgui/hooks/use-server-health";
import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { cn } from "@/agentgui/lib/utils";
import type { ReasoningEffort, TransportMode } from "@/agentgui/types/workspace";

const efforts: ReasoningEffort[] = ["low", "medium", "high"];
const transports: { id: TransportMode; label: string; detail: string }[] = [
  {
    id: "server",
    label: "Rust server",
    detail: "Stream chat runs from the local metamind kernel.",
  },
  {
    id: "mock",
    label: "Local mock",
    detail: "Scripted stream for UI work without a backend.",
  },
];

export function AgentSettingsDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const model = useWorkspaceStore((state) => state.model);
  const setModel = useWorkspaceStore((state) => state.setModel);
  const reasoningEffort = useWorkspaceStore((state) => state.reasoningEffort);
  const setReasoningEffort = useWorkspaceStore(
    (state) => state.setReasoningEffort
  );
  const transport = useWorkspaceStore((state) => state.transport);
  const setTransport = useWorkspaceStore((state) => state.setTransport);
  const systemPrompt = useWorkspaceStore((state) => state.systemPrompt);
  const setSystemPrompt = useWorkspaceStore((state) => state.setSystemPrompt);
  const { health } = useServerHealth();

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Agent settings</DialogTitle>
          <DialogDescription>
            Transport, reasoning effort and the system prompt used for every
            turn. With the Rust backend the model comes from `OPENAI_MODEL`.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <label
              className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase"
              htmlFor="settings-model"
            >
              Model
            </label>
            <Input
              disabled={transport === "server"}
              id="settings-model"
              onChange={(event) => setModel(event.target.value)}
              placeholder="gpt-4o-mini"
              value={
                transport === "server" ? (health?.model ?? model) : model
              }
            />
            <p className="font-mono text-[10px] text-muted-foreground">
              {transport === "server"
                ? health
                  ? `set by OPENAI_MODEL · backend: ${health.backend}`
                  : "waiting for the backend to report its model"
                : "used by the local mock transport"}
            </p>
          </div>

          <div className="space-y-1.5">
            <span className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
              Reasoning effort
            </span>
            <div className="flex gap-1">
              {efforts.map((effort) => (
                <button
                  className={cn(
                    "flex-1 rounded-lg border px-2 py-1.5 text-[12px] capitalize transition-all",
                    reasoningEffort === effort
                      ? "active-nav-item font-medium"
                      : "border-border bg-surface text-muted-foreground hover:text-foreground"
                  )}
                  key={effort}
                  onClick={() => setReasoningEffort(effort)}
                  type="button"
                >
                  {effort}
                </button>
              ))}
            </div>
          </div>

          <div className="space-y-1.5">
            <span className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
              Transport
            </span>
            <div className="grid gap-1.5 sm:grid-cols-2">
              {transports.map((option) => (
                <button
                  className={cn(
                    "rounded-lg border p-2 text-left transition-all",
                    transport === option.id
                      ? "border-primary/30 bg-accent/40 shadow-elev-1"
                      : "border-border bg-surface hover:border-border"
                  )}
                  key={option.id}
                  onClick={() => setTransport(option.id)}
                  type="button"
                >
                  <span className="block text-[12px] font-medium">
                    {option.label}
                  </span>
                  <span className="mt-0.5 block text-[10.5px] leading-snug text-muted-foreground">
                    {option.detail}
                  </span>
                </button>
              ))}
            </div>
          </div>

          <div className="space-y-1.5">
            <label
              className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase"
              htmlFor="settings-system"
            >
              System prompt
            </label>
            <Textarea
              className="min-h-24 resize-none text-[12px] leading-relaxed"
              id="settings-system"
              onChange={(event) => setSystemPrompt(event.target.value)}
              value={systemPrompt}
            />
          </div>
        </div>

        <DialogFooter>
          <Button onClick={() => onOpenChange(false)} variant="ghost">
            Close
          </Button>
          <Button onClick={() => onOpenChange(false)}>Apply</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
