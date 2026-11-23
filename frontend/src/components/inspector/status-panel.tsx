import type { StatusView } from "@/lib/wire/inspector.ts";

import { PromptBudgetRow } from "./prompt-budget-row.tsx";
import {
  ActionsBlock,
  AgentRuns,
  AutomationBlock,
  ConversationBlock,
  FrameBlock,
  InterjectionBlock,
  KnowledgeBlock,
  MemoryPanel,
  MetamindBlock,
  OperationList,
  PlanBlock,
  SessionBlock,
  StatusVariables,
  VariablesBlock,
} from "./status-blocks.tsx";
import { TopicHeader } from "./topic-header.tsx";
import { UnavailableRow } from "./section.tsx";

export function StatusPanel({
  view,
  onOpenFiles,
  onOpenKnowledge,
  onOpenPlan,
  onOpenTrace,
  onCancelAction,
}: {
  view: StatusView;
  onOpenFiles?: () => void;
  onOpenKnowledge?: () => void;
  onOpenPlan?: () => void;
  onOpenTrace?: (traceId: string) => void;
  onCancelAction?: (actionId: string) => void;
}) {
  const actions = [...view.actions.scheduled, ...view.actions.recent];
  const unavailable = Object.entries(view.unavailable).filter(
    ([key]) => key !== "budget" && key !== "memory" && key !== "automation",
  );
  return (
    <div className="min-h-0 flex-1 overflow-y-auto" data-status-panel>
      <TopicHeader view={view} />
      <PromptBudgetRow view={view} />
      {unavailable.map(([section, reason]) => (
        <div className="px-3 pt-2" key={section}>
          <UnavailableRow section={section} reason={reason} />
        </div>
      ))}
      <SessionBlock onOpenFiles={onOpenFiles} view={view} />
      <FrameBlock view={view} />
      <ConversationBlock view={view} />
      <MetamindBlock />
      <KnowledgeBlock onOpen={onOpenKnowledge} view={view} />
      <PlanBlock onOpen={onOpenPlan} view={view} />
      <ActionsBlock items={actions} nextRunAt={view.actions.nextRunAt} onCancel={onCancelAction} />
      <AutomationBlock view={view} />
      <InterjectionBlock view={view} />
      <MemoryPanel view={view} />
      <StatusVariables view={view} />
      <AgentRuns agents={view.agents} onOpen={onOpenTrace} />
      <OperationList view={view} />
      <VariablesBlock view={view} />
    </div>
  );
}
