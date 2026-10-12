import { expect, it } from "vitest";

import { countMatching } from "@/lib/logs.ts";
import { initialTranscript, reduceTranscript, type TranscriptState } from "@/lib/transcript/reducer.ts";
import { pointsForZoom } from "@/lib/trees/layout.ts";
import { heapTree, visibleRows } from "@/lib/trees/model.ts";
import type { ChatFrame } from "@/lib/wire/chat.ts";

it("reduces a chat frame onto a 1000-row transcript within one frame budget", () => {
  let state: TranscriptState = initialTranscript;
  for (let i = 0; i < 999; i += 1) {
    state = reduceTranscript(state, {
      seq: i + 1,
      frame: { type: "final_response", response_id: `r${i}`, text: "fixture", suggestions: [] } as ChatFrame,
    });
  }
  const started = performance.now();
  state = reduceTranscript(state, {
    seq: 1000,
    frame: { type: "final_response", response_id: "r999", text: "fixture", suggestions: [] } as ChatFrame,
  });
  const elapsed = performance.now() - started;
  expect(elapsed).toBeLessThan(16);
  expect(state.items).toHaveLength(1000);
});

it("filters a million-row log by severity inside the keystroke budget", () => {
  const started = performance.now();
  const errors = countMatching("error", 1_000_000);
  const elapsed = performance.now() - started;
  expect(elapsed).toBeLessThan(100);
  expect(errors).toBe(20_000);
});

it("expands a 20k-node tree inside the expand budget", () => {
  const nodes = heapTree(20_000);
  const expanded = new Set(nodes.map((node) => node.id));
  const started = performance.now();
  const rows = visibleRows(nodes, expanded);
  const elapsed = performance.now() - started;
  expect(elapsed).toBeLessThan(50);
  expect(rows).toHaveLength(20_000);
});

it("lays out a 5k-node map zoom step with depth-of-field culling", () => {
  const nodes = heapTree(5_000);
  const started = performance.now();
  const visible = pointsForZoom(nodes, 0.4);
  const elapsed = performance.now() - started;
  expect(elapsed).toBeLessThan(50);
  expect(visible.length).toBeGreaterThan(0);
  expect(visible.every((point) => point.depth <= 2)).toBe(true);
});
