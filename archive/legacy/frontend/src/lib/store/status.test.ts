import { describe, expect, it } from "vitest";

import { statusFixture } from "@/lib/inspector/__fixtures__/status.ts";
import {
  applyStatusFrame,
  coalesceStatusFrames,
  emptySessionStatusState,
  mergeStatusView,
} from "./status.ts";

describe("status reducer", () => {
  it("ignores duplicate and stale revisions", () => {
    const state = applyStatusFrame(emptySessionStatusState, statusFixture(5), 100);
    const duplicate = applyStatusFrame(state, statusFixture(5), 200);
    const stale = applyStatusFrame(state, statusFixture(4), 300);
    expect(duplicate).toBe(state);
    expect(stale).toBe(state);
    expect(state.view?.revision).toBe(5);
    expect(state.lastFrameAt).toBe(100);
  });

  it("accepts a newer revision and clears staleness", () => {
    const state = applyStatusFrame(emptySessionStatusState, statusFixture(5), 100);
    state.stale = true;
    const next = applyStatusFrame(state, statusFixture(6), 400);
    expect(next.view?.revision).toBe(6);
    expect(next.lastFrameAt).toBe(400);
    expect(next.stale).toBe(false);
    expect(next.connection).toBe("live");
  });

  it("keeps unchanged section references", () => {
    const first = statusFixture(5);
    const second = statusFixture(6);
    second.data = { ...second.data, indexPending: 4 };
    const merged = mergeStatusView(first, second);
    expect(merged).not.toBe(first);
    expect(merged.state).toBe(first.state);
    expect(merged.session).toBe(first.session);
    expect(merged.data).not.toBe(first.data);
    expect(merged.data.indexPending).toBe(4);
  });

  it("coalesces a burst to the highest revision", () => {
    const frame = coalesceStatusFrames([statusFixture(7), statusFixture(9), statusFixture(8)]);
    expect(frame?.revision).toBe(9);
    expect(coalesceStatusFrames([])).toBeNull();
  });
});
