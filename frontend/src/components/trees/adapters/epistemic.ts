import { remoteTree } from "@/components/trees/adapters/remote.ts";

export const epistemicAdapter = remoteTree("epistemic", "Epistemic", ["claim", "evidence", "assumption", "contradiction"], ["label", "kind", "mono"]);
