import { fetchTree } from "@/lib/api/trees.ts";
import type { TreeSnapshot } from "@/lib/trees/model.ts";

export type TreeAdapter = {
  name: string;
  title: string;
  kinds: string[];
  columns: string[];
  load: () => Promise<TreeSnapshot>;
};

export function remoteTree(name: string, title: string, kinds: string[], columns: string[]): TreeAdapter {
  return {
    name,
    title,
    kinds,
    columns,
    load: () => fetchTree(name),
  };
}
