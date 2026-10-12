import { remoteTree } from "@/components/trees/adapters/remote.ts";

export const libraryAdapter = remoteTree("library", "Library", ["technique", "policy", "frame", "skill", "case"], ["label", "kind", "mono"]);
