import { render } from "@testing-library/react";
import { expect, it } from "vitest";

import { LogDocument } from "@/components/docs/log-doc.tsx";

it("virtualizes a million log rows", () => {
  const previous = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get() {
      return this instanceof HTMLElement && this.hasAttribute("data-log-scroll") ? 384 : 22;
    },
  });
  try {
    const { container } = render(<LogDocument count={1_000_000} />);
    const rows = container.querySelectorAll("[data-log-row]");
    expect(container.querySelector("[data-log-count]")?.getAttribute("data-log-count")).toBe("1000000");
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeLessThan(80);
  } finally {
    if (previous) Object.defineProperty(HTMLElement.prototype, "offsetHeight", previous);
  }
});
