import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { XIcon } from "lucide-react";

import { cancelAction, fetchActions } from "@/lib/api/inspector.ts";
import type { ScheduledActionRow } from "@/lib/wire/inspector.ts";
import { errorMessage, until } from "@/lib/inspector/format.ts";
import { useNow } from "@/hooks/use-now.ts";

import { Chip, UnavailableRow } from "./section.tsx";

function ActionRow({
  action,
  now,
  onCancel,
  cancelling,
}: {
  action: ScheduledActionRow;
  now: number;
  onCancel: (id: string) => void;
  cancelling: boolean;
}) {
  return (
    <li className="border-border flex items-center gap-1.5 rounded-sm border px-2 py-1.5">
      <span className="min-w-0 flex-1 truncate text-[12px]">{action.kind}</span>
      <Chip>{action.status}</Chip>
      <span className="w-16 shrink-0 text-right font-mono text-[10px] text-muted-foreground">
        {action.dueAt ? until(action.dueAt, now) : `${action.runs} runs`}
      </span>
      <button
        aria-label={`Cancel ${action.kind}`}
        className="text-muted-foreground hover:text-destructive disabled:opacity-40"
        disabled={cancelling || action.status === "cancelled" || action.status === "done"}
        onClick={() => onCancel(action.actionId)}
        title="Cancel action"
        type="button"
      >
        <XIcon className="size-3" />
      </button>
    </li>
  );
}

export function ActionsPanel({ sessionId }: { sessionId: string | null }) {
  const queryClient = useQueryClient();
  const now = useNow();
  const actions = useQuery({
    queryKey: ["inspector", "actions", sessionId],
    queryFn: () => fetchActions(sessionId ?? undefined),
  });
  const cancel = useMutation({
    mutationFn: cancelAction,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["inspector", "actions"] });
      void queryClient.invalidateQueries({ queryKey: ["inspector", "status"] });
    },
  });

  return (
    <div className="flex min-h-0 flex-col" data-panel="actions">
      <div className="border-border/70 flex items-center gap-2 border-b px-3 py-2">
        <span className="flex-1 text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          Actions
        </span>
        <Chip>{actions.data?.scheduled.length ?? 0} scheduled</Chip>
        <Chip>next {actions.data?.nextRunAt ? until(actions.data.nextRunAt, now) : "not set"}</Chip>
      </div>
      <div className="min-h-0 overflow-y-auto p-2">
        {actions.isError ? (
          <UnavailableRow section="actions" reason={errorMessage(actions.error)} />
        ) : null}
        {cancel.isError ? (
          <UnavailableRow section="cancel" reason={errorMessage(cancel.error)} />
        ) : null}
        {actions.isPending ? <p className="text-[12px] text-muted-foreground">Loading actions…</p> : null}
        {actions.data ? (
          <>
            <p className="mb-1 text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
              Scheduled
            </p>
            {actions.data.scheduled.length ? (
              <ul className="mb-3 flex flex-col gap-1">
                {actions.data.scheduled.map((action) => (
                  <ActionRow
                    action={action}
                    cancelling={cancel.isPending}
                    key={action.actionId}
                    now={now}
                    onCancel={(id) => cancel.mutate(id)}
                  />
                ))}
              </ul>
            ) : (
              <p className="mb-3 text-[12px] text-muted-foreground">No scheduled actions.</p>
            )}
            <p className="mb-1 text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
              Recent
            </p>
            {actions.data.recent.length ? (
              <ul className="flex flex-col gap-1">
                {actions.data.recent.map((action) => (
                  <ActionRow
                    action={action}
                    cancelling={cancel.isPending}
                    key={action.actionId}
                    now={now}
                    onCancel={(id) => cancel.mutate(id)}
                  />
                ))}
              </ul>
            ) : (
              <p className="text-[12px] text-muted-foreground">No recent actions.</p>
            )}
          </>
        ) : null}
      </div>
    </div>
  );
}
