import { expect, it } from "vitest";

import { chartCsv } from "@/components/charts/export.ts";

it("exports chart series as csv", () => {
  const csv = chartCsv([{ name: "Actual", points: [{ x: "1", y: 2 }] }]);
  expect(csv).toBe("name,x,y\nActual,1,2\n");
});
