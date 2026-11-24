import { useEffect } from "react";

import { useWorkspaceStore } from "@/agentgui/lib/store/workspace-store";
import { usePrefsStore } from "@/lib/store/prefs.ts";

interface ShortcutHandlers {
  onFocusComposer?: () => void;
}

/** App-level keyboard shortcuts: layout, sidebar, new session, composer focus. */
export function useKeyboardShortcuts({ onFocusComposer }: ShortcutHandlers = {}) {
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const meta = event.metaKey || event.ctrlKey;
      if (!meta) return;

      const store = useWorkspaceStore.getState();

      switch (event.key.toLowerCase()) {
        case "b":
          event.preventDefault();
          store.toggleSidebarCompact();
          break;
        case "\\":
          event.preventDefault();
          store.setPaneLayout(store.paneLayout === "two" ? "chat" : "two");
          break;
        case "j":
          event.preventDefault();
          store.setPaneLayout(
            store.paneLayout === "workspace" ? "two" : "workspace"
          );
          break;
        case "n":
          event.preventDefault();
          store.createSession();
          break;
        case "k":
          event.preventDefault();
          onFocusComposer?.();
          break;
        case "`":
          if (!event.shiftKey) {
            event.preventDefault();
            usePrefsStore.getState().toggleLlmLogCollapsed();
          }
          break;
        default:
          break;
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onFocusComposer]);
}
