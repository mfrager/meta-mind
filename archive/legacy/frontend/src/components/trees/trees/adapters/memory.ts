import { remoteTree } from "@/components/trees/adapters/remote.ts";

export const memoryAdapter = remoteTree("memory", "Memory", ["topic", "episode", "mistake", "procedure"], ["label", "kind", "mono"]);
