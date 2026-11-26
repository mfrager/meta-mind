import { remoteTree } from "@/components/trees/adapters/remote.ts";

export const budgetsAdapter = remoteTree("budgets", "Budgets", ["budget", "bucket", "allocation", "run"], ["label", "kind", "mono"]);
