import { Trash2Icon } from "lucide-react";
import { useEffect, useState } from "react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/agentgui/components/ui/dialog.tsx";
import { deleteConsoleSession } from "@/lib/api/sessions.ts";
import { errorMessage, shortId } from "@/lib/inspector/format.ts";

export type DeletableSession = {
  sessionId: string;
  topic?: string | null;
  lifecycle?: string;
};

export function DeleteSessionDialog({
  session,
  onOpenChange,
  onDeleted,
}: {
  session: DeletableSession | null;
  onOpenChange: (open: boolean) => void;
  onDeleted: (sessionId: string) => void;
}) {
  const [mode, setMode] = useState<"soft" | "purge">("soft");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (session) {
      setMode("soft");
      setConfirmation("");
      setError(null);
    }
  }, [session]);

  const canDelete = Boolean(session) && (mode === "soft" || confirmation.trim() === session?.sessionId);

  return (
    <Dialog onOpenChange={onOpenChange} open={Boolean(session)}>
      <DialogContent className="max-w-md" showCloseButton={!busy}>
        <DialogHeader>
          <DialogTitle>Delete session?</DialogTitle>
          <DialogDescription>
            {mode === "soft"
              ? "Soft delete hides the session from the inspector. Its RDF rows, traces and logs stay on disk."
              : "Purge ends the session and removes its RDF rows, trace manifests, log entries and on-demand directory."}
          </DialogDescription>
        </DialogHeader>
        {session ? (
          <div className="flex flex-col gap-2 text-[12px]">
            <p className="font-mono text-[11px]">{session.topic ?? "Untitled session"}</p>
            <p className="font-mono text-[10px] text-muted-foreground">
              {session.sessionId} · {session.lifecycle}
            </p>
            <label className="flex items-center gap-2">
              <input checked={mode === "soft"} onChange={() => setMode("soft")} type="radio" />
              Soft delete (default)
            </label>
            <label className="flex items-center gap-2">
              <input checked={mode === "purge"} onChange={() => setMode("purge")} type="radio" />
              Hard purge
            </label>
            {mode === "purge" ? (
              <label className="flex flex-col gap-1">
                <span className="text-[11px] text-muted-foreground">
                  Type <span className="font-mono">{session.sessionId}</span> to confirm.
                </span>
                <input
                  aria-label="Type the session id to confirm purge"
                  className="border-input bg-background h-8 rounded-sm border px-2 font-mono text-[11px]"
                  onChange={(event) => setConfirmation(event.target.value)}
                  value={confirmation}
                />
              </label>
            ) : null}
            {session.lifecycle === "active" ? (
              <p className="text-[11px] text-amber-700 dark:text-amber-300">
                The session is active; it will be ended before deletion.
              </p>
            ) : null}
            {error ? <p className="text-[11px] text-red-700 dark:text-red-300">{error}</p> : null}
          </div>
        ) : null}
        <DialogFooter>
          <button
            className="text-muted-foreground hover:text-foreground text-[12px]"
            disabled={busy}
            onClick={() => onOpenChange(false)}
            type="button"
          >
            Cancel
          </button>
          <button
            className="bg-destructive text-destructive-foreground inline-flex items-center gap-1 rounded-sm px-2.5 py-1 text-[12px] disabled:opacity-50"
            disabled={!canDelete || busy}
            onClick={() => {
              if (!session) return;
              setBusy(true);
              setError(null);
              void deleteConsoleSession(session.sessionId, mode, session.lifecycle)
                .then(() => {
                  onDeleted(session.sessionId);
                  onOpenChange(false);
                })
                .catch((failure: unknown) => setError(errorMessage(failure)))
                .finally(() => setBusy(false));
            }}
            type="button"
          >
            <Trash2Icon className="size-3" />
            {busy ? "Deleting…" : mode === "purge" ? `Purge ${shortId(session?.sessionId ?? "")}` : "Soft delete"}
          </button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
