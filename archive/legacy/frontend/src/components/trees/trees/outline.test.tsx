import { render } from "@testing-library/react";
import { expect, it } from "vitest";

import { OutlineView } from "@/components/trees/outline.tsx";
import { heapTree, visibleRows } from "@/lib/trees/model.ts";

it("virtualizes a 20k-node fixture", () => {
  const previous = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get() {
      return this instanceof HTMLElement && this.hasAttribute("data-tree-scroll") ? 384 : 28;
    },
  });
  try {
    const nodes = heapTree(20_000);
    const rows = visibleRows(nodes, new Set(nodes.map((node) => node.id)));
    const { container } = render(
      <OutlineView rows={rows} nodes={nodes} focusId="n0" expanded={new Set(nodes.map((node) => node.id))} onFocus={() => undefined} onToggle={() => undefined} />,
    );
    const rendered = container.querySelectorAll("[data-tree-row]");
    expect(container.querySelector("[data-tree-count]")?.getAttribute("data-tree-count")).toBe("20000");
    expect(rendered.length).toBeGreaterThan(0);
    expect(rendered.length).toBeLessThan(80);
  } finally {
    if (previous) Object.defineProperty(HTMLElement.prototype, "offsetHeight", previous);
  }
});
