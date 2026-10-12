import { useEffect, useState } from "react";

import { ChatHeader } from "@/components/chat/chat-header.tsx";
import { Composer } from "@/components/chat/composer.tsx";
import { SuggestionChips } from "@/components/chat/chat-form.tsx";
import type { ImageAttachment } from "@/components/chat/image-chooser.tsx";
import { TranscriptView } from "@/components/chat/transcript-view.tsx";
import { answerInput, branchChat, createSession, openChat, regenerateChat, resumeChat, stopChat } from "@/lib/api/chat.ts";
import { useChatPending } from "@/lib/store/chat-pending.ts";
import { useSessionStore } from "@/lib/store/session.ts";
import { applyControllerFocus, useSideStore, type LastAction } from "@/lib/store/side.ts";
import { initialTranscript, reduceFrames, type TranscriptItem, type TranscriptState } from "@/lib/transcript/reducer.ts";
import { suggestionsOf, visibleItems } from "@/lib/transcript/selectors.ts";

type FreshSession = { id: string; title: string; requestId?: string };

// React Strict Mode runs the mount effect twice. Both passes share one POST so
// the discarded pass does not spend a second model introduction.
let opening: Promise<FreshSession> | null = null;
let openingUsers = 0;

function beginSession(): Promise<FreshSession> {
  openingUsers += 1;
  if (!opening) opening = createSession();
  return opening;
}

function endSessionAttempt() {
  openingUsers -= 1;
  if (openingUsers === 0) {
    queueMicrotask(() => {
      if (openingUsers === 0) opening = null;
    });
  }
}

export function ChatPane() {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [state, setState] = useState<TranscriptState>(initialTranscript);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const pending = useChatPending((store) => store.text);

  useEffect(() => {
    let cancelled = false;
    const sessionPromise = beginSession();
    void sessionPromise
      .then(async (session) => {
        if (cancelled) return;
        setSessionId(session.id);
        useSessionStore.getState().setSession(session.id, session.title);
        if (!session.requestId) return;
        const frames = await resumeChat(session.requestId, 1);
        if (cancelled) return;
        setState((current) => reduceFrames(current, frames));
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setNotice(error instanceof Error ? error.message : "The chat session could not start.");
      });
    return () => {
      cancelled = true;
      endSessionAttempt();
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
    if (useSessionStore.getState().title === "Chat") {
      useSessionStore.getState().setTitle(query.trim().slice(0, 60));
    }
    setBusy(true);
    setNotice(null);
    try {
      const side = useSideStore.getState();
      const frames = await openChat({
        sessionId: sessionId ?? undefined,
        query,
        refs,
        attachments: attachments.map(({ name, type, size, caption }) => ({ name, type, size, caption })),
        pane:
          side.mode === "document" && side.doc
            ? { id: side.doc.id, title: side.doc.title }
            : { id: "state", title: "State" },
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
    void applyControllerFocus(state.focus);
  }, [state.focus]);

  useEffect(() => {
    const action = lastActionOf(state.items);
    useSideStore.getState().setLastAction(action);
    if (action?.ok && (action.name.startsWith("being.") || action.name === "budget.show")) {
      useSideStore.getState().bump();
    }
  }, [state.items]);

  const followSuggestion = (text: string) => {
    void send(text, [], []);
  };

  const items = visibleItems(state);

  return (
    <section className="flex h-full min-h-0 flex-col" data-chat-pane>
      <ChatHeader requestId={state.requestId} onStop={() => void stop()} onRegenerate={() => void regenerate()} onBranch={() => void branch()} />
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

function lastActionOf(items: TranscriptItem[]): LastAction | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (!item || item.kind !== "turn") continue;
    const op = [...item.ops].reverse().find((call) => call.output !== undefined);
    if (!op) continue;
    const output = op.output as { summary?: unknown; ok?: unknown } | null;
    const summary = output && typeof output.summary === "string" ? output.summary : "";
    return { name: op.name, ok: op.success ?? output?.ok === true, summary };
  }
  return null;
}
