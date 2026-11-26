import type { ReactNode } from "react";
import { CircleAlert, Info, TriangleAlert } from "lucide-react";

import { Button } from "@/components/ui/actions.tsx";
import { cn } from "@/lib/utils.ts";

const tones = {
  info: "border-azure-200 bg-azure-50 text-azure-900",
  warning: "border-amber-200 bg-amber-50 text-amber-950",
  error: "border-red-200 bg-red-50 text-red-950",
  positive: "border-green-200 bg-green-50 text-green-950",
};

export function Alert({ tone = "info", title, children }: { tone?: keyof typeof tones; title: string; children?: ReactNode }) {
  return (
    <div role="status" data-token={`alert ${tone}`} className={cn("rounded-xs border px-3 py-2", tones[tone])}>
      <p className="text-h3">{title}</p>
      {children ? <div className="text-small mt-1">{children}</div> : null}
    </div>
  );
}

export function Banner({
  tone = "info",
  children,
  onDismiss,
}: {
  tone?: keyof typeof tones;
  children: ReactNode;
  onDismiss?: () => void;
}) {
  const Icon = tone === "error" ? CircleAlert : tone === "warning" ? TriangleAlert : Info;
  return (
    <div role={tone === "error" ? "alert" : "status"} data-token={`banner ${tone}`} className={cn("flex items-start gap-2 border-b px-3 py-2", tones[tone])}>
      <Icon className="mt-0.5 size-3.5 shrink-0" aria-hidden />
      <div className="text-small min-w-0 flex-1">{children}</div>
      {onDismiss ? (
        <button type="button" className="text-small underline" onClick={onDismiss}>
          Dismiss
        </button>
      ) : null}
    </div>
  );
}

export function InlineNote({ tone = "note", children }: { tone?: "tip" | "note" | "warning"; children: ReactNode }) {
  return (
    <p data-token={`inline-note ${tone}`} className="text-small text-muted-foreground border-border rounded-xs border-l-2 border-l-amber-500 px-2 py-1">
      <span className="micro-label mr-2">{tone}</span>
      {children}
    </p>
  );
}

export function EmptyState({ sentence, action }: { sentence: string; action: string }) {
  return (
    <div data-token="empty-state" className="flex flex-col items-start gap-2">
      <svg aria-hidden width="80" height="48" className="opacity-[0.08]">
        <path d="M4 40 L40 8 L76 40 Z" fill="currentColor" />
        <path d="M8 40 H72" stroke="currentColor" />
      </svg>
      <p className="text-body">{sentence}</p>
      <Button>{action}</Button>
    </div>
  );
}
export function Empty({ title, action }: { title: string; action?: string }) {
  return (
    <div data-token="empty" className="flex flex-col items-start gap-2 py-4">
      <p className="text-body">{title}</p>
      {action ? <Button variant="outline">{action}</Button> : null}
    </div>
  );
}

export function ErrorState({
  message,
  reason,
  correlationId,
}: {
  message: string;
  reason: string;
  correlationId?: string;
}) {
  return (
    <div data-token="error-state" role="alert" className="border-border rounded-xs border border-l-[3px] border-l-red-500 p-3">
      <p className="text-h3">{message}</p>
      <pre className="bg-muted mt-2 overflow-auto rounded-xs p-2 font-mono text-value">{reason}</pre>
      <div className="mt-2 flex gap-2">
        <Button size="sm">Retry</Button>
        <Button size="sm" variant="outline">
          Open logs
        </Button>
      </div>
      {correlationId ? <p className="text-small text-muted-foreground mt-2 font-mono">{correlationId}</p> : null}
    </div>
  );
}
