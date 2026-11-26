import { useMemo, useState } from "react";

import { AgentstreamDock } from "@/components/agentstream/AgentstreamDock.tsx";
import { Composer } from "@/components/chat/composer.tsx";
import { TranscriptView } from "@/components/chat/transcript-view.tsx";
import { useAgentstream } from "@/hooks/use-agentstream.ts";
import { startTurn } from "@/lib/api/turns.ts";
import { useAgentstreamStore } from "@/lib/store/agentstream.ts";
import { deriveTranscript } from "@/lib/transcript/reducer.ts";

export function ChatPane({
  sessionId,
  title,
  onStarted,
}: {
  sessionId: string | null;
  title: string;
  onStarted: (sessionId: string, text: string) => void;
}) {
  useAgentstream(sessionId);
  const records = useAgentstreamStore((state) => state.records);
  const deltas = useAgentstreamStore((state) => state.deltas);
  const items = useMemo(() => deriveTranscript(records, deltas), [records, deltas]);
  const [sending, setSending] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const send = async (text: string) => {
    setSending(true);
    setNotice(null);
    try {
      const accepted = await startTurn({ sessionId, text });
      onStarted(accepted.sessionId, text);
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "The turn could not start.");
    } finally {
      setSending(false);
    }
  };

  return (
    <section className="flex h-full min-h-0 flex-col" data-chat-pane>
      <header className="border-border flex h-10 shrink-0 items-center gap-2 border-b px-3">
        <h1 className="min-w-0 flex-1 truncate text-sm font-medium">{title}</h1>
        <span className="text-muted-foreground font-mono text-xs">{sessionId ? sessionId.slice(0, 12) : "new"}</span>
      </header>
      {notice ? (
        <p role="status" className="text-small px-3 py-1 text-red-700">
          {notice}
        </p>
      ) : null}
      <div className="min-h-0 flex-1 overflow-hidden">
        <TranscriptView items={items} />
      </div>
      <Composer onSend={(text) => void send(text)} disabled={sending} />
      <AgentstreamDock />
    </section>
  );
}
