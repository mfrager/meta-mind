import { useEffect, useState } from "react";

import { checkHealth } from "@/agentgui/services/agent-transport";
import type { ServerHealth } from "@/agentgui/types/workspace";

/** Polls the Rust backend once on mount and reports its capability. */
export function useServerHealth() {
  const [health, setHealth] = useState<ServerHealth | null>(null);
  const [checking, setChecking] = useState(true);

  useEffect(() => {
    let active = true;
    const run = async () => {
      const result = await checkHealth();
      if (!active) return;
      setHealth(result);
      setChecking(false);
    };
    void run();
    const timer = setInterval(run, 30_000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, []);

  return { health, checking };
}
