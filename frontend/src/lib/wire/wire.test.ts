import { expect, test } from "vitest";

import chatFrames from "./fixtures/chat_frames.json" with { type: "json" };
import focusDoc from "./fixtures/focus_doc.json" with { type: "json" };
import healthFixture from "./fixtures/health.json" with { type: "json" };
import type { ChatFrame } from "./chat.ts";
import type { FocusDoc } from "./focus.ts";
import { WIRE_VERSION, type HealthEnvelope } from "./index.ts";

test("the wire version is 1", () => {
  expect(WIRE_VERSION).toBe(1);
});

test("the health fixture is an envelope at this wire version", () => {
  const health = healthFixture as HealthEnvelope;
  expect(health.ok).toBe(true);
  expect(health.wireVersion).toBe(WIRE_VERSION);
  const unavailable = health.kernel.services.find((service) => service.state === "unavailable");
  expect(unavailable?.reason).toBeTruthy();
});

test("chat fixtures include a resumable run frame", () => {
  const frames = chatFrames as ChatFrame[];
  const run = frames.find((frame) => frame.type === "run");
  expect(run?.type).toBe("run");
  if (run?.type !== "run") {
    return;
  }
  expect(run.requestId).toHaveLength(26);
  expect(run.sessionId).toHaveLength(26);
});

test("a focus document that cannot load carries its reason", () => {
  const doc = focusDoc as FocusDoc;
  expect(doc.status).toBe("unavailable");
  expect(doc.reason).toContain("document pane");
});
