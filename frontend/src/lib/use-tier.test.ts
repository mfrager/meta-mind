import { expect, test } from "vitest";

import { tierForWidth } from "./use-tier.ts";

test("the four widths land on the four shell tiers", () => {
  expect(tierForWidth(1400)).toBe("wide");
  expect(tierForWidth(1280)).toBe("wide");
  expect(tierForWidth(1279)).toBe("medium");
  expect(tierForWidth(1024)).toBe("medium");
  expect(tierForWidth(1023)).toBe("narrow");
  expect(tierForWidth(768)).toBe("narrow");
  expect(tierForWidth(767)).toBe("monitor");
  expect(tierForWidth(600)).toBe("monitor");
});
