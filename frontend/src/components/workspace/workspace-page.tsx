import { useCallback, useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";

import { ChatPane } from "@/components/chat/chat-pane.tsx";
import { fetchSessions } from "@/lib/api/agentstream.ts";
import type { SessionIndexRecord } from "@/lib/wire/agentstream.ts";

/**
 * One route: session picker, transcript, composer, and the Agentstream dock.
 */
export function WorkspacePage() {
  const params = useParams();
  const navigate = useNavigate();
  const sessionId = params.sessionId ?? null;
  const [sessions, setSessions] = useState<SessionIndexRecord[]>([]);
  const [titles, setTitles] = useState<Record<string, string>>({});
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await fetchSessions();
      setSessions(list.sessions);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "The session list could not load.");
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // A fresh server has no sessions; the first composer send creates one. When
  // sessions exist, the bare route opens the most recent one.
  useEffect(() => {
    if (creating || sessionId || sessions.length === 0) return;
    navigate(`/chat/${sessions[0]?.sessionId ?? ""}`, { replace: true });
  }, [creating, sessionId, sessions, navigate]);

  const title =
    (sessionId ? titles[sessionId] : undefined) ??
    sessions.find((session) => session.sessionId === sessionId)?.title ??
    (sessionId ? `Session ${sessionId.slice(0, 8)}` : "New session");

  return (
    <div className="bg-background flex h-dvh flex-col" data-workspace>
      <header className="border-border flex h-11 shrink-0 items-center gap-2 border-b px-3">
        <strong className="text-sm">Metamind</strong>
        <select
          aria-label="Session"
          data-session-picker
          className="border-input bg-background text-small h-7 max-w-64 rounded-md border px-2"
          value={sessionId ?? ""}
          onChange={(event) => {
            const next = event.target.value;
            setCreating(false);
            if (next) navigate(`/chat/${next}`);
            else {
              setCreating(true);
              navigate("/");
            }
          }}
        >
          <option value="">New session</option>
          {sessions.map((session) => (
            <option key={session.sessionId} value={session.sessionId}>
              {(session.title ?? titles[session.sessionId] ?? `Session ${session.sessionId.slice(0, 8)}`).slice(0, 60)}
            </option>
          ))}
        </select>
        <button
          type="button"
          data-new-session
          className="border-input text-small hover:bg-muted h-7 rounded-md border px-2"
          onClick={() => {
            setCreating(true);
            navigate("/");
          }}
        >
          New
        </button>
        {error ? <span className="text-small text-red-700">{error}</span> : null}
      </header>
      <main className="min-h-0 flex-1">
        <ChatPane
          sessionId={sessionId}
          title={title}
          onStarted={(id, text) => {
            setCreating(false);
            setTitles((current) => ({ ...current, [id]: text.slice(0, 60) }));
            setSessions((current) =>
              current.some((session) => session.sessionId === id)
                ? current
                : [
                    {
                      sessionId: id,
                      userId: "operator",
                      title: null,
                      createdAt: Date.now(),
                      lastSeq: 0,
                      updatedAt: Date.now(),
                    },
                    ...current,
                  ],
            );
            navigate(`/chat/${id}`);
            void refresh();
          }}
        />
      </main>
    </div>
  );
}
