import { remoteTree } from "@/components/trees/adapters/remote.ts";

export const ontologyAdapter = remoteTree("ontology", "Ontology", ["class", "property", "shape", "named-graph"], ["label", "kind", "mono"]);
