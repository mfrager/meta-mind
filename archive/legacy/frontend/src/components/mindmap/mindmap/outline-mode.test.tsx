import { render } from "@testing-library/react";
import axe from "axe-core";
import { expect, it } from "vitest";

import { OutlineMode } from "@/components/mindmap/outline-mode.tsx";
import type { TreeNode } from "@/lib/trees/model.ts";

const nodes: TreeNode[] = [
  { id: "being", parentId: null, kind: "identity", label: "Being", capabilities: [] },
  { id: "being-goals", parentId: "being", kind: "goal", label: "Goals", capabilities: [] },
  { id: "being-invariants", parentId: "being", kind: "invariant", label: "Invariants", capabilities: [] },
];

it("exposes the map as a screen-reader tree", async () => {
  const { container } = render(<OutlineMode nodes={nodes} />);
  const items = container.querySelectorAll("[role='treeitem']");
  expect(items).toHaveLength(3);
  expect(items[0]?.getAttribute("aria-level")).toBe("1");
  expect(items[1]?.getAttribute("aria-level")).toBe("2");
  const results = await axe.run(container, {
    rules: {
      "color-contrast": { enabled: false },
      "page-has-heading-one": { enabled: false },
    },
  });
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
});
