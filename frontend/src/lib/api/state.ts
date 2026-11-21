import { useEffect } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";

import { useHealth } from "@/lib/api/health.ts";

export type StateBoard = {
  kernel: {
    state: string;
    reason?: string | null;
    ok_services: string;
    llm_reason: string;
    services: { name: string; state: string; reason?: string | null }[];
  };
  being:
    | {
        ok: true;
        self_description: string;
        version: string;
        invariants: string[];
        goal_count: number;
        voice: {
          warmth: number;
          directness: number;
          curiosity: number;
          playfulness: number;
          formality: number;
        };
      }
    | { ok: false; reason: string };
  goals: { goals?: { id: string; description: string; priority: number; status: string }[] };
  budgets: { rows?: { kind: string; period: string; limit: number; spent: number }[]; reason?: string };
  last_action: { name: string; ok: boolean; summary: string } | null;
};

export async function fetchState(): Promise<StateBoard> {
  const response = await fetch("/api/state");
  if (!response.ok) {
    throw new Error(`GET /api/state returned ${response.status}`);
  }
  return response.json() as Promise<StateBoard>;
}

/**
 * Refetch when the health poll moves, and when a tool asks the board to reload.
 *
 * The query key is deliberately stable. Putting `health.dataUpdatedAt` in the key
 * gave every poll a fresh cache entry, so the board had no data for a moment and
 * the whole panel flashed through its loading state every five seconds. A refetch
 * against one key updates the rendered board in place instead.
 */
export function useStateBoard(revision: number) {
  const health = useHealth();
  const query = useQuery({
    queryKey: ["console-state"],
    queryFn: fetchState,
    // A revision change keeps the previous rows on screen while the new ones load.
    placeholderData: keepPreviousData,
  });

  const { refetch } = query;
  const healthAt = health.dataUpdatedAt;
  useEffect(() => {
    // `0` means "no poll has answered yet": the first mount already fetched.
    if (healthAt > 0 || revision > 0) void refetch();
  }, [healthAt, revision, refetch]);

  return query;
}
