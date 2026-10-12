export type Shortcut =
  | "palette"
  | "composer"
  | "document"
  | "chat"
  | "swap"
  | "sidebar"
  | "fullscreen"
  | "mode"
  | "save"
  | "cheatsheet"
  | "quick-switch"
  | "duplicate"
  | "close-tab"
  | "dismiss";

export type ShortcutInput = {
  key: string;
  meta: boolean;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  /** True when the event target is a text field. */
  typing: boolean;
  /** True when the event target is the chat composer. */
  composer: boolean;
};

function normalizeKey(key: string): string {
  if (key === "Escape") return "Escape";
  return key.toLowerCase();
}

/**
 * Bindings never fire while typing, except Escape, ⌘K, and ⌘S in the composer
 * (send). Tree and mind-map keys stay unbound until those surfaces exist.
 */
export function shortcutFromEvent(input: ShortcutInput): Shortcut | null {
  if (input.alt) return null;
  const key = normalizeKey(input.key);
  const mod = input.meta || input.ctrl;

  if (key === "Escape") return "dismiss";
  if (mod && key === "k") return "palette";
  if (input.typing) {
    if (input.composer && mod && !input.shift && key === "s") return "save";
    return null;
  }

  if (!mod) return null;
  if (key === "l" && !input.shift) return "composer";
  if (key === "1") return "document";
  if (key === "2") return "chat";
  if (key === "\\") return "swap";
  if (key === "b" && !input.shift) return "sidebar";
  if (key === "f" && input.shift) return "fullscreen";
  if (key === "e" && !input.shift) return "mode";
  if (key === "s" && !input.shift) return "save";
  if (key === "/") return "cheatsheet";
  if (key === "g" && !input.shift) return "quick-switch";
  if (key === "d" && !input.shift) return "duplicate";
  if (key === "w" && !input.shift) return "close-tab";
  return null;
}

export function isTextEntry(target: EventTarget | null): { typing: boolean; composer: boolean } {
  if (!(target instanceof HTMLElement)) return { typing: false, composer: false };
  const composer = target.dataset.composer === "true";
  const typing =
    composer ||
    target.isContentEditable ||
    target.tagName === "INPUT" ||
    target.tagName === "TEXTAREA" ||
    target.tagName === "SELECT";
  return { typing, composer };
}

export function modLabel(): string {
  if (typeof navigator === "undefined") return "Ctrl";
  return /Mac|iPhone|iPad/.test(navigator.userAgent) ? "⌘" : "Ctrl";
}
