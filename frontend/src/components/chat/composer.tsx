import { useEffect, useMemo, useState } from "react";

import { ImageChooser, type ImageAttachment } from "@/components/chat/image-chooser.tsx";
import { Button } from "@/components/ui/button.tsx";
import { COMMANDS } from "@/lib/commands.ts";
import { useChatRefs } from "@/lib/store/chat-refs.ts";

export function Composer({
  onSend,
  disabled,
}: {
  onSend: (text: string, refs: string[], attachments: ImageAttachment[]) => void;
  disabled?: boolean;
}) {
  const [draft, setDraft] = useState("");
  const [refs, setRefs] = useState<string[]>([]);
  const storedRefs = useChatRefs((state) => state.refs);
  const clearStored = useChatRefs((state) => state.clear);
  const shownRefs = [...storedRefs, ...refs.filter((ref) => !storedRefs.includes(ref))];
  const [images, setImages] = useState<ImageAttachment[]>([]);
  const [imagesOpen, setImagesOpen] = useState(false);
  const slash = draft.startsWith("/") ? draft.slice(1).toLowerCase() : null;
  const matches = useMemo(
    () => (slash === null ? [] : COMMANDS.filter((command) => command.id.includes(slash) || command.title.toLowerCase().includes(slash)).slice(0, 8)),
    [slash],
  );

  const send = () => {
    const text = draft.trim();
    if (!text || disabled) return;
    onSend(text, shownRefs, images);
    setDraft("");
    setRefs([]);
    clearStored();
    setImages([]);
  };

  useEffect(() => {
    const onSend = () => send();
    window.addEventListener("mm-composer-send", onSend);
    return () => window.removeEventListener("mm-composer-send", onSend);
  });

  return (
    <form
      className="space-y-2 border-t border-border p-3"
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
    >
      {matches.length ? (
        <ul data-slash-menu role="listbox" className="rounded-md border border-border bg-background">
          {matches.map((command) => (
            <li key={command.id}>
              <button
                type="button"
                className="w-full px-2 py-1 text-left text-sm hover:bg-muted"
                onClick={() => setDraft(`${command.argv} `)}
              >
                {command.title}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      {shownRefs.length ? (
        <ul className="flex flex-wrap gap-1">
          {shownRefs.map((ref) => (
            <li key={ref} className="rounded-md bg-muted px-1.5 py-0.5 font-mono text-xs">
              {ref}
            </li>
          ))}
        </ul>
      ) : null}
      <textarea
        id="composer"
        data-composer="true"
        rows={3}
        value={draft}
        placeholder="Ask MetaMind. ⌘↵ sends."
        className="w-full resize-none rounded-md border border-border bg-background px-2 py-1 text-sm"
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            send();
          }
        }}
      />
      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={disabled || !draft.trim()}>
          Send
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            const ref = window.prompt("Reference, for example @map:ontology#ont-thing");
            if (ref?.trim()) setRefs([...refs, ref.trim()]);
          }}
        >
          @ ref
        </Button>
        <Button type="button" variant="outline" onClick={() => setImagesOpen((open) => !open)}>
          Image
        </Button>
      </div>
      {imagesOpen ? <ImageChooser images={images} onChange={setImages} /> : null}
    </form>
  );
}
