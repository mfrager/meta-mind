import { useEffect, useState } from "react";

import { ImageChooser, type ImageAttachment } from "@/components/chat/image-chooser.tsx";
import { Button } from "@/components/ui/button.tsx";
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
      data-composer-form
      className="border-border space-y-2 border-t p-3"
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
    >
      {shownRefs.length ? (
        <ul className="flex flex-wrap gap-1">
          {shownRefs.map((ref) => (
            <li key={ref} className="bg-muted rounded-md px-1.5 py-0.5 font-mono text-xs">
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
        placeholder="Ask Metamind…"
        className="border-border w-full resize-none rounded-md border bg-background px-2 py-1 text-sm"
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

export function focusComposer() {
  document.querySelector<HTMLElement>("#composer")?.focus();
}
