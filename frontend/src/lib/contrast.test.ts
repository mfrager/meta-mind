/// <reference types="node" />
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { expect, it } from "vitest";

import { contrastRatio, parseOklch, type Oklch } from "./contrast.ts";

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "../index.css"), "utf8");

function block(source: string, selector: string): string {
  const start = source.indexOf(selector);
  if (start < 0) throw new Error(`missing ${selector}`);
  const open = source.indexOf("{", start);
  const close = source.indexOf("}", open);
  return source.slice(open + 1, close);
}

function token(source: string, name: string): Oklch {
  const match = source.match(new RegExp(`${name}:\\s*(oklch\\([^)]*\\))`));
  const parsed = match ? parseOklch(match[1] ?? "") : null;
  if (!parsed) throw new Error(`missing ${name}`);
  return parsed;
}

const hues = ["azure", "teal", "violet", "amber", "red", "green"] as const;
const light = block(css, "\n:root {");
const dark = block(css, "\n.dark {");

function hue(name: string, step: number): Oklch {
  return token(css, `--hue-${name}-${step}`);
}

/** Tint recipes from design/planning/ui/02 §3.1. Micro-labels are 10px bold, so the floor is 3:1. */
const tintPairs: [fg: number, bg: number][] = [
  [700, 50],
  [700, 100],
  [300, 950],
];

it("keeps every hue ramp's text-on-tint pairs at the micro-label floor", () => {
  for (const name of hues) {
    for (const [fg, bg] of tintPairs) {
      const ratio = contrastRatio(hue(name, fg), hue(name, bg));
      expect(ratio, `${name} ${fg} on ${bg} is ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(3);
    }
  }
});

it("keeps body text and button text at 4.5:1 in both themes", () => {
  const pairs: [string, string, string][] = [
    ["light", "--foreground", "--background"],
    ["light", "--card-foreground", "--card"],
    ["light", "--primary-foreground", "--primary"],
    ["light", "--muted-foreground", "--background"],
    ["dark", "--foreground", "--background"],
    ["dark", "--card-foreground", "--card"],
    ["dark", "--primary-foreground", "--primary"],
    ["dark", "--muted-foreground", "--background"],
  ];
  for (const [theme, fg, bg] of pairs) {
    const source = theme === "light" ? light : dark;
    const ratio = contrastRatio(token(source, fg), token(source, bg));
    expect(ratio, `${theme} ${fg} on ${bg} is ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(4.5);
  }
});
