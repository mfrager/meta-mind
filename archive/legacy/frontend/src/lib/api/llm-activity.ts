import { fetchJson, withQuery } from "@/lib/api/client.ts";
import type { LlmActivitySnapshot } from "@/lib/wire/llm_activity.ts";

/** Load the retained activity ring, newest first bounded by `limit`. */
export function fetchLlmActivity(
  params: {
    from?: number;
    limit?: number;
    kind?: string;
    sessionId?: string;
  } = {},
): Promise<LlmActivitySnapshot> {
  return fetchJson<LlmActivitySnapshot>(withQuery("/api/llm-activity", params));
}

/** The resumable live tail. `from` is one past the last seen event. */
export function llmActivityStreamUrl(from: number): string {
  return withQuery("/api/llm-activity/stream", { from });
}
