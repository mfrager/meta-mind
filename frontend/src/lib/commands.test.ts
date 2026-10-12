import { expect, test } from "vitest";

import { COMMANDS, COMMAND_GROUPS } from "./commands.ts";
import { DOCUMENTS } from "./nav.ts";

const CLI_IDS = [
  "doctor",
  "replay",
  "graph",
  "logs",
  "codex",
  "llm",
  "being",
  "memory",
  "epistemic",
  "library",
  "skill",
  "case",
  "experience",
  "policy",
  "frame",
  "episode",
  "decide",
  "firewall",
  "compare",
  "risk",
  "calibration",
  "conformance",
  "tool",
  "action",
  "verify",
  "mcp",
  "calibrate",
  "meta",
  "regression",
  "changeset",
  "sandbox",
  "promote",
  "reject",
  "budget",
  "pi",
  "audit",
  "loop",
  "self-model",
  "debt",
  "gc",
  "module",
  "design",
] as const;

test("the palette lists the 42 top-level CLI commands", () => {
  expect(COMMANDS).toHaveLength(42);
  expect(new Set(COMMANDS.map((command) => command.id)).size).toBe(42);
  expect([...COMMANDS.map((command) => command.id)].sort()).toEqual([...CLI_IDS].sort());
});

test("every command names a real group and a real document", () => {
  for (const command of COMMANDS) {
    expect(COMMAND_GROUPS).toContain(command.group);
    expect(command.argv.startsWith(command.id)).toBe(true);
    expect(command.summary.length).toBeGreaterThan(0);
    if (command.doc) {
      expect(DOCUMENTS[command.doc]).toBeTruthy();
    }
  }
});
