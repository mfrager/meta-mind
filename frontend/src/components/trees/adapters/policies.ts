import { remoteTree } from "@/components/trees/adapters/remote.ts";

export const policiesAdapter = remoteTree("policies", "Policies", ["policy", "generation", "rule", "authorization"], ["label", "kind", "mono"]);
