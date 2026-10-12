import { render } from "@testing-library/react";
import { expect, it } from "vitest";

import { RadialEngine } from "@/components/mindmap/radial-engine.tsx";
import { pointsForZoom, layoutRadial } from "@/lib/trees/layout.ts";
import { heapTree } from "@/lib/trees/model.ts";

it("lays out 5k nodes with depth-of-field culling under half zoom", () => {
  const nodes = heapTree(5_000);
  const started = performance.now();
  const visible = pointsForZoom(nodes, 0.4);
  const elapsed = performance.now() - started;
  expect(elapsed).toBeLessThan(250);
  expect(visible.length).toBeGreaterThan(0);
  expect(visible.every((point) => point.depth <= 2)).toBe(true);
  expect(layoutRadial(nodes, 0.4).find((point) => point.depth === 6)?.lod).toBe("dot");
  const { container } = render(<RadialEngine points={visible} total={nodes.length} />);
  expect(Number(container.querySelector("[data-map-rendered]")?.getAttribute("data-map-rendered"))).toBe(visible.length);
  expect(container.querySelectorAll("[data-map-node]").length).toBe(visible.length);
  expect(container.querySelector("[data-lod='dot']")).toBeNull();
});
