import { remoteTree } from "@/components/trees/adapters/remote.ts";

export const changesetsAdapter = remoteTree("changesets", "Change sets", ["change-set", "file", "hunk", "op"], ["label", "kind", "mono"]);
