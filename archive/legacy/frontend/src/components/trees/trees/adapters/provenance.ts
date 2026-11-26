import { remoteTree } from "@/components/trees/adapters/remote.ts";

export const provenanceAdapter = remoteTree("provenance", "Provenance", ["event", "span", "actor", "write"], ["label", "kind", "mono"]);
