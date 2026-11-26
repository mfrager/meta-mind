import { fetchJson } from "@/lib/api/client.ts";
import type { TurnAccepted } from "@/lib/wire/agentstream.ts";

/** Start one turn. A null session creates a session on the server. */
export function startTurn(input: {
  sessionId: string | null;
  text: string;
  userId?: string | null;
}): Promise<TurnAccepted> {
  return fetchJson<TurnAccepted>("/api/turns", {
    method: "POST",
    body: JSON.stringify({
      sessionId: input.sessionId,
      text: input.text,
      userId: input.userId ?? null,
    }),
  });
}
