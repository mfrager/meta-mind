import { useEffect, useState } from "react";

import { useNavigate } from "react-router-dom";

import { ChatHeader } from "@/components/chat/chat-header.tsx";
import { Composer } from "@/components/chat/composer.tsx";
import { SuggestionChips } from "@/components/chat/chat-form.tsx";
import type { ImageAttachment } from "@/components/chat/image-chooser.tsx";
import { TranscriptView } from "@/components/chat/transcript-view.tsx";
import { Button } from "@/components/ui/button.tsx";
import { answerInput, branchChat, createSession, openChat, regenerateChat, resumeChat, stopChat } from "@/lib/api/chat.ts";
import { FOCUS_FRAME } from "@/lib/focus/fixture.ts";
import { useDocuments } from "@/lib/store/documents.ts";
import { useChatPending } from "@/lib/store/chat-pending.ts";
import { usePaneStore } from "@/lib/store/pane.ts";
import { initialTranscript, reduceFrames, type TranscriptState } from "@/lib/transcript/reducer.ts";
import { suggestionsOf, visibleItems } from "@/lib/transcript/selectors.ts";
import type { FocusDoc } from "@/lib/wire/focus.ts";

export function ChatPane() {
  const applyFrame = useDocuments((state) => state.applyFrame);
  const navigate = useNavigate();
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [title, setTitle] = useState("Chat");
  const [state, setState] = useState<TranscriptState>(initialTranscript);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const pending = useChatPending((store) => store.text);

  useEffect(() => {
    let cancelled = false;
    void createSession()
      .then((session) => {
        if (cancelled) return;
        setSessionId(session.id);
        setTitle(session.title);
      })
      .catch((error: unknown) => setNotice(error instanceof Error ? error.message : "The chat session could not start."));
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!sessionId || !pending) return;
    const text = useChatPending.getState().take();
    if (text) void send(text, [], []);
    // send closes over sessionId; pending is the trigger.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId, pending]);

  const send = async (query: string, refs: string[], attachments: ImageAttachment[]) => {
    setBusy(true);
    setNotice(null);
    try {
      const frames = await openChat({
        sessionId: sessionId ?? undefined,
        query,
        refs,
        attachments: attachments.map(({ name, type, size, caption }) => ({ name, type, size, caption })),
      });
      setState((current) => reduceFrames(current, frames));
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "The chat run failed.");
    } finally {
      setBusy(false);
    }
  };

  const answer = async (id: string, value: string) => {
    if (!sessionId) return;
    const frames = await answerInput(sessionId, id, value);
    setState((current) => reduceFrames(current, frames));
  };

  const stop = async () => {
    if (!state.requestId) return;
    const result = await stopChat(state.requestId);
    setNotice(result.reason ?? result.state);
  };

  const regenerate = async () => {
    if (!state.requestId) return;
    setNotice(null);
    const frames = await regenerateChat(state.requestId);
    setState(reduceFrames(initialTranscript, frames));
  };

  const branch = async () => {
    if (!sessionId) return;
    const next = await branchChat(sessionId);
    setSessionId(next.sessionId);
    const frames = await resumeChat(next.requestId, 1);
    setState(reduceFrames(initialTranscript, frames));
  };

  useEffect(() => {
    const doc = state.focus;
    if (!doc || typeof doc !== "object" || !("id" in doc)) return;
    applyFrame({ type: "focus", doc: doc as FocusDoc });
    usePaneStore.getState().setActivePane("document");
  }, [applyFrame, state.focus]);

  const followSuggestion = (text: string) => {
    if (text === "Open the budgets tree") {
      navigate("/?doc=budgets");
      return;
    }
    if (text === "Open the being map") {
      navigate("/map/being");
      return;
    }
    void send(text, [], []);
  };

  const items = visibleItems(state);

  return (
    <section className="flex h-full min-h-0 flex-col" data-chat-pane>
      <ChatHeader title={title} requestId={state.requestId} onStop={() => void stop()} onRegenerate={() => void regenerate()} onBranch={() => void branch()} />
      <div className="flex items-center justify-between px-3 pt-2">
        <p className="text-xs text-muted-foreground">Wire v1</p>
        <Button type="button" variant="outline" data-focus-fixture onClick={() => applyFrame(FOCUS_FRAME)}>
          Open focus fixture
        </Button>
      </div>
      {notice ? (
        <p role="status" className="px-3 text-sm text-muted-foreground">
          {notice}
        </p>
      ) : null}
      <div className="min-h-0 flex-1 overflow-auto">
        <TranscriptView items={items} onAnswer={(id, value) => void answer(id, value)} />
      </div>
      <SuggestionChips suggestions={suggestionsOf(state)} onPick={followSuggestion} />
      <Composer disabled={busy || !sessionId} onSend={(text, refs, attachments) => void send(text, refs, attachments)} />
    </section>
  );
}
