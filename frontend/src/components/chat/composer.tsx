import { useState } from "react";

import { Button } from "@/components/ui/actions.tsx";

export function Composer({
  onSend,
  disabled,
}: {
  onSend: (text: string) => void;
  disabled?: boolean;
}) {
  const [draft, setDraft] = useState("");

  const send = () => {
    const text = draft.trim();
    if (!text || disabled) return;
    onSend(text);
    setDraft("");
  };

  return (
    <form
      data-composer-form
      className="border-border flex items-end gap-2 border-t p-3"
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
    >
      <textarea
        id="composer"
        data-composer="true"
        aria-label="Message"
        rows={2}
        value={draft}
        placeholder="Ask Metamind…"
        className="border-input bg-background text-body min-h-[var(--composer-min)] flex-1 resize-none rounded-md border px-2 py-1.5"
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            send();
          }
        }}
      />
      <Button type="submit" disabled={disabled || !draft.trim()}>
        Send
      </Button>
    </form>
  );
}
