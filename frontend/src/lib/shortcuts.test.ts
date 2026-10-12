import { expect, test } from "vitest";

import { shortcutFromEvent, type ShortcutInput } from "./shortcuts.ts";

function press(partial: Partial<ShortcutInput> & { key: string }): ShortcutInput {
  return {
    meta: false,
    ctrl: false,
    shift: false,
    alt: false,
    typing: false,
    composer: false,
    ...partial,
  };
}

test("command shortcuts stay quiet while typing", () => {
  expect(shortcutFromEvent(press({ key: "k", meta: true, typing: true }))).toBe("palette");
  expect(shortcutFromEvent(press({ key: "Escape", typing: true }))).toBe("dismiss");
  expect(shortcutFromEvent(press({ key: "b", ctrl: true, typing: true }))).toBeNull();
  expect(shortcutFromEvent(press({ key: "e", meta: true, typing: true }))).toBeNull();
  expect(shortcutFromEvent(press({ key: "s", meta: true, typing: true, composer: true }))).toBe("save");
  expect(shortcutFromEvent(press({ key: "s", meta: true, typing: true }))).toBeNull();
});

test("the shell map resolves the milestone bindings", () => {
  expect(shortcutFromEvent(press({ key: "k", ctrl: true }))).toBe("palette");
  expect(shortcutFromEvent(press({ key: "1", meta: true }))).toBe("document");
  expect(shortcutFromEvent(press({ key: "2", ctrl: true }))).toBe("chat");
  expect(shortcutFromEvent(press({ key: "\\", meta: true }))).toBe("swap");
  expect(shortcutFromEvent(press({ key: "b", ctrl: true }))).toBe("sidebar");
  expect(shortcutFromEvent(press({ key: "e", meta: true }))).toBe("mode");
  expect(shortcutFromEvent(press({ key: "F", meta: true, shift: true }))).toBe("fullscreen");
  expect(shortcutFromEvent(press({ key: "/", ctrl: true }))).toBe("cheatsheet");
  expect(shortcutFromEvent(press({ key: "k" }))).toBeNull();
  expect(shortcutFromEvent(press({ key: "b", alt: true }))).toBeNull();
});
