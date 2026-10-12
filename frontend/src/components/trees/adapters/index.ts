import type { TreeAdapter } from "@/components/trees/adapters/remote.ts";
import { beingAdapter } from "@/components/trees/adapters/being.ts";
import { budgetsAdapter } from "@/components/trees/adapters/budgets.ts";
import { changesetsAdapter } from "@/components/trees/adapters/changesets.ts";
import { epistemicAdapter } from "@/components/trees/adapters/epistemic.ts";
import { libraryAdapter } from "@/components/trees/adapters/library.ts";
import { memoryAdapter } from "@/components/trees/adapters/memory.ts";
import { modulesAdapter } from "@/components/trees/adapters/modules.ts";
import { ontologyAdapter } from "@/components/trees/adapters/ontology.ts";
import { policiesAdapter } from "@/components/trees/adapters/policies.ts";
import { provenanceAdapter } from "@/components/trees/adapters/provenance.ts";

const ADAPTERS: Record<string, TreeAdapter> = {
  ontology: ontologyAdapter,
  being: beingAdapter,
  memory: memoryAdapter,
  epistemic: epistemicAdapter,
  library: libraryAdapter,
  modules: modulesAdapter,
  policies: policiesAdapter,
  provenance: provenanceAdapter,
  changesets: changesetsAdapter,
  budgets: budgetsAdapter,
};

export function adapterFor(name: string): TreeAdapter {
  const adapter = ADAPTERS[name];
  if (!adapter) throw new Error(`no tree ${name}`);
  return adapter;
}
