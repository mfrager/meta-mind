import { expect, it } from "vitest";

import {
  heapTree,
  initialTreeState,
  parseTreeMapRef,
  reduceTree,
  roundTripRef,
  treeMapRef,
  visibleRows,
  type TreeSnapshot,
} from "@/lib/trees/model.ts";

const sample = [
  { id: "ont", parentId: null, kind: "class", label: "Ontology", capabilities: ["add-child"] as const },
  { id: "ont-thing", parentId: "ont", kind: "class", label: "Thing", capabilities: ["rename", "move", "delete"] as const },
  { id: "ont-name", parentId: "ont-thing", kind: "property", label: "name", capabilities: ["rename", "move", "delete"] as const },
];

function loaded() {
  return reduceTree(initialTreeState, { type: "load", nodes: sample.map((node) => ({ ...node, capabilities: [...node.capabilities] })) });
}

it("renames, moves, and deletes with undo and redo", () => {
  const start = loaded();
  const renamed = reduceTree(start, { type: "rename", id: "ont-thing", label: "Entity" });
  expect(renamed.nodes.find((node) => node.id === "ont-thing")?.label).toBe("Entity");
  const undone = reduceTree(renamed, { type: "undo" });
  expect(undone.nodes.find((node) => node.id === "ont-thing")?.label).toBe("Thing");
  const redone = reduceTree(undone, { type: "redo" });
  expect(redone.nodes.find((node) => node.id === "ont-thing")?.label).toBe("Entity");

  const moved = reduceTree(start, { type: "move", id: "ont-name", targetId: "ont", position: "inside" });
  expect(moved.nodes.find((node) => node.id === "ont-name")?.parentId).toBe("ont");
  expect(reduceTree(moved, { type: "undo" }).nodes.find((node) => node.id === "ont-name")?.parentId).toBe("ont-thing");

  const removed = reduceTree(start, { type: "remove", id: "ont-name" });
  expect(removed.nodes.some((node) => node.id === "ont-name")).toBe(false);
  expect(reduceTree(removed, { type: "undo" }).nodes.some((node) => node.id === "ont-name")).toBe(true);
});

it("walks the tree from the keyboard", () => {
  const start = loaded();
  const down = reduceTree(start, { type: "key", key: "ArrowDown" });
  expect(down.focusId).toBe("ont-thing");
  const expanded = reduceTree(down, { type: "key", key: "ArrowRight" });
  expect(expanded.expanded).toContain("ont-thing");
  const home = reduceTree(expanded, { type: "key", key: "Home" });
  expect(home.focusId).toBe("ont");
  const end = reduceTree(home, { type: "key", key: "End" });
  expect(end.focusId).toBe("ont-name");
});

it("round-trips a tree node through a map chat reference", () => {
  const tree: TreeSnapshot = { name: "ontology", title: "Ontology", fixture: true, nodes: sample.map((node) => ({ ...node, capabilities: [...node.capabilities] })) };
  expect(roundTripRef(tree, "ont-thing")?.label).toBe("Thing");
  expect(parseTreeMapRef(treeMapRef("ontology", "ont-thing"))).toEqual({ tree: "ontology", nodeId: "ont-thing" });
});

it("keeps a 20k fixture addressable", () => {
  const nodes = heapTree(20_000);
  const rows = visibleRows(nodes, new Set(nodes.map((node) => node.id)));
  expect(rows).toHaveLength(20_000);
  expect(rows.some((row) => row.node.id === "n19999")).toBe(true);
});
