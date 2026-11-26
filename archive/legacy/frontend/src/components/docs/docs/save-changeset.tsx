import { useState } from "react";

import { createChangeset, saveDraft } from "@/lib/api/focus.ts";
import { Button } from "@/components/ui/actions.tsx";
import { Field, Input, Textarea } from "@/components/ui/controls.tsx";
import { Stepper } from "@/components/ui/navigation.tsx";
import type { FocusDoc } from "@/lib/wire/focus.ts";

export function SaveChangeset({
  doc,
  content,
  onClose,
  onSaved,
}: {
  doc: FocusDoc;
  content: string;
  onClose: () => void;
  onSaved: (doc: FocusDoc, id: string) => void;
}) {
  const snapshot = (doc.snapshot ?? {}) as { path?: string };
  const [step, setStep] = useState(0);
  const [summary, setSummary] = useState("Record the console note as a change set");
  const [reason, setReason] = useState("");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const path = snapshot.path || "docs/console-note.md";

  async function submit() {
    setError("");
    if (reason.trim().length < 10) {
      setError("A reason needs at least 10 characters.");
      return;
    }
    setPending(true);
    try {
      await saveDraft(doc.id, content);
      const created = await createChangeset({ docId: doc.id, reason, summary, path, content });
      if (created.doc) onSaved(created.doc, created.id);
      else setError("The kernel recorded no document.");
    } catch (err) {
      setError(err instanceof Error ? err.message : "The change set was refused.");
    } finally {
      setPending(false);
    }
  }

  return (
    <form
      data-changeset-wizard
      className="border-border bg-card flex flex-col gap-3 rounded-lg border p-3"
      onSubmit={(event) => {
        event.preventDefault();
        if (step < 2) setStep(step + 1);
        else void submit();
      }}
    >
      <Stepper steps={["Summary", "Diff", "Reason"]} current={step} />
      {step === 0 ? (
        <Field label="Summary" htmlFor="cs-summary">
          <Input id="cs-summary" value={summary} onChange={(event) => setSummary(event.target.value)} />
        </Field>
      ) : null}
      {step === 1 ? (
        <pre className="bg-muted overflow-auto rounded-xs p-2 font-mono text-value">{`modify ${path}\n${content}`}</pre>
      ) : null}
      {step === 2 ? (
        <Field label="Reason" htmlFor="cs-reason" required error={error || undefined}>
          <Textarea id="cs-reason" value={reason} onChange={(event) => setReason(event.target.value)} />
        </Field>
      ) : null}
      {error && step !== 2 ? <p className="text-small text-red-700" role="alert">{error}</p> : null}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" onClick={onClose}>Cancel</Button>
        <Button type="submit" loading={pending}>{step < 2 ? "Next" : "Record change set"}</Button>
      </div>
    </form>
  );
}
