import { useEffect } from "react";
import { useLocation, useNavigate } from "react-router-dom";

import { focusComposer } from "@/components/chat/composer.tsx";
import { hrefFor } from "@/lib/nav.ts";
import { isTextEntry, shortcutFromEvent } from "@/lib/shortcuts.ts";
import { useFocusStore } from "@/lib/store/focus.ts";
import { usePaneStore } from "@/lib/store/pane.ts";
import { useViewportTier } from "@/lib/use-tier.ts";

const LATER = "Document tabs arrive with the document pane.";

export function KeyboardMap() {
  const navigate = useNavigate();
  const location = useLocation();
  const tier = useViewportTier();

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target;
      const { typing, composer } = isTextEntry(target);
      const shortcut = shortcutFromEvent({
        key: event.key,
        meta: event.metaKey,
        ctrl: event.ctrlKey,
        shift: event.shiftKey,
        alt: event.altKey,
        typing,
        composer,
      });
      if (!shortcut) return;

      const inOverlay =
        target instanceof HTMLElement && Boolean(target.closest("dialog, [cmdk-dialog], [role='dialog']"));
      if (shortcut === "dismiss" && inOverlay) return;

      const pane = usePaneStore.getState();
      const focus = useFocusStore.getState();

      if (pane.paletteOpen && shortcut !== "palette" && shortcut !== "dismiss") return;

      if (shortcut === "dismiss") {
        if (pane.paletteOpen) {
          event.preventDefault();
          pane.setPaletteOpen(false);
        } else if (pane.shortcutsOpen) {
          event.preventDefault();
          pane.setShortcutsOpen(false);
        } else if (pane.sidebarSheet) {
          event.preventDefault();
          pane.setSheet(false);
        } else if (focus.notice) {
          event.preventDefault();
          focus.clearNotice();
        } else if (pane.fullscreen) {
          event.preventDefault();
          pane.toggleFullscreen();
        }
        return;
      }

      event.preventDefault();

      const focusLater = (selector: string) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            document.querySelector<HTMLElement>(selector)?.focus();
          });
        });
      };

      switch (shortcut) {
        case "palette":
        case "composer":
          pane.setPaletteOpen(false);
          pane.setActivePane("chat");
          if (location.pathname === "/dev/catalog") {
            navigate("/");
            focusLater("#composer");
          } else {
            focusComposer();
          }
          break;
        case "document":
          pane.setActivePane("document");
          if (location.pathname === "/dev/catalog") {
            navigate("/");
            focusLater("[data-pane='document']");
          } else {
            document.querySelector<HTMLElement>("[data-pane='document']")?.focus();
          }
          break;
        case "chat":
          pane.setActivePane("chat");
          if (location.pathname === "/dev/catalog") {
            navigate("/chat");
            focusLater("[data-pane='chat']");
          } else {
            document.querySelector<HTMLElement>("[data-pane='chat']")?.focus();
          }
          break;
        case "swap":
          if (tier === "narrow" || tier === "monitor") {
            pane.setActivePane(pane.activePane === "document" ? "chat" : "document");
          }
          pane.toggleSwap();
          break;
        case "sidebar":
          if (tier === "wide") pane.toggleSidebar();
          else pane.toggleSheet();
          break;
        case "fullscreen":
          if (location.pathname === "/dev/catalog") navigate("/");
          pane.toggleFullscreen();
          break;
        case "mode":
          if (tier === "monitor") {
            focus.setNotice("Editing needs a wider window. The console stays read-only here.");
            pane.setActivePane("chat");
          } else {
            focus.toggleMode();
            pane.setActivePane("document");
            if (location.pathname === "/dev/catalog") navigate("/");
          }
          break;
        case "save":
          if (composer) {
            window.dispatchEvent(new Event("mm-composer-send"));
          } else {
            window.dispatchEvent(new Event("mm-save-draft"));
            pane.setActivePane("document");
          }
          break;
        case "cheatsheet":
          pane.setShortcutsOpen(!pane.shortcutsOpen);
          break;
        case "quick-switch":
          window.dispatchEvent(new Event("mm-quick-switch"));
          pane.setActivePane("document");
          break;
        case "close-tab":
          window.dispatchEvent(new Event("mm-close-tab"));
          pane.setActivePane("document");
          break;
        case "duplicate":
          focus.setNotice(LATER);
          pane.setActivePane("document");
          if (location.pathname === "/dev/catalog") navigate(hrefFor("overview"));
          break;
        default:
          break;
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [location.pathname, navigate, tier]);

  return null;
}
