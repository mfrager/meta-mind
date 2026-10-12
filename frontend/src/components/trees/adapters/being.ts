import { remoteTree } from "@/components/trees/adapters/remote.ts";

export const beingAdapter = remoteTree("being", "Being", ["identity", "invariant", "goal", "subgoal"], ["label", "kind", "mono"]);
