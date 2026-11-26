import { remoteTree } from "@/components/trees/adapters/remote.ts";

export const modulesAdapter = remoteTree("modules", "Modules", ["module", "version", "capability", "hook"], ["label", "kind", "mono"]);
