/// <reference types="node" />
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { expect, it } from "vitest";

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "../index.css"), "utf8");

it("zeroes motion tokens and the pulse when the user prefers reduced motion", () => {
  expect(css).toContain("@media (prefers-reduced-motion: reduce)");
  expect(css).toContain("--t-fast: 0ms");
  expect(css).toContain("--t-base: 0ms");
  expect(css).toContain("--t-slow: 0ms");
  expect(css).toMatch(/\.mm-pulse\s*\{\s*animation:\s*none;/);
});

it("strengthens borders when the user prefers more contrast", () => {
  expect(css).toContain("@media (prefers-contrast: more)");
  expect(css).toContain("--border: var(--border-strong)");
});

it("prints the transcript and the side pane, and leaves the shell chrome off the page", () => {
  const print = css.slice(css.indexOf("@media print"));
  expect(print).toContain("[data-sidebar-panel]");
  expect(print).toContain("[data-topbar]");
  expect(print).toContain("[data-statusbar]");
  expect(print).toContain("[data-composer-form]");
  expect(print).toContain("[data-chat-pane],\n  [data-side-pane]");
  expect(print).not.toMatch(/\[data-chat-pane\][\s\S]*display:\s*none/);
});
