import { useQuery } from "@tanstack/react-query";

import type { HealthEnvelope } from "@/lib/wire/index.ts";

export const healthKey = ["health"] as const;

async function fetchHealth(): Promise<HealthEnvelope> {
  const response = await fetch("/api/health");
  if (!response.ok) {
    throw new Error(`GET /api/health returned ${response.status}`);
  }
  return response.json() as Promise<HealthEnvelope>;
}

export function useHealth() {
  return useQuery({
    queryKey: healthKey,
    queryFn: fetchHealth,
    refetchInterval: 5_000,
  });
}
