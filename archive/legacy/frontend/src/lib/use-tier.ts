import { useEffect, useState } from "react";

/** Width tiers from design/planning/ui/03 §8. */
export type Tier = "wide" | "medium" | "narrow" | "monitor";

export function tierForWidth(width: number): Tier {
  if (width >= 1280) return "wide";
  if (width >= 1024) return "medium";
  if (width >= 768) return "narrow";
  return "monitor";
}

function readTier(): Tier {
  if (typeof window === "undefined") return "wide";
  return tierForWidth(window.innerWidth);
}

export function useViewportTier(): Tier {
  const [tier, setTier] = useState<Tier>(readTier);

  useEffect(() => {
    const onResize = () => setTier(readTier());
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  return tier;
}
