import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { Bell, Moon, Search, Sun } from "lucide-react";

import { useHealth } from "@/lib/api/health.ts";
import { NAV_SECTIONS, hrefFor, useFocusedDocument } from "@/lib/nav.ts";
import { modLabel } from "@/lib/shortcuts.ts";
import { usePaneStore } from "@/lib/store/pane.ts";
import { usePrefsStore } from "@/lib/store/prefs.ts";
import { cn } from "@/lib/utils.ts";

export function TopBar() {
  const focused = useFocusedDocument();
  const setPaletteOpen = usePaneStore((state) => state.setPaletteOpen);
  const resetLayout = usePaneStore((state) => state.resetLayout);
  const toggleSwap = usePaneStore((state) => state.toggleSwap);
  const setShortcutsOpen = usePaneStore((state) => state.setShortcutsOpen);
  const theme = usePrefsStore((state) => state.theme);
  const toggleTheme = usePrefsStore((state) => state.toggleTheme);
  const density = usePrefsStore((state) => state.density);
  const toggleDensity = usePrefsStore((state) => state.toggleDensity);
  const health = useHealth();
  const [menu, setMenu] = useState(false);
  const [notes, setNotes] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const mod = modLabel();
  const section = NAV_SECTIONS.find((entry) => entry.section === focused.section);
  const sectionHref = hrefFor(section?.ids[0] ?? "overview");

  useEffect(() => {
    if (!menu && !notes) return;
    const onPointer = (event: PointerEvent) => {
      if (!(event.target instanceof Node)) return;
      if (menuRef.current?.contains(event.target)) return;
      setMenu(false);
      setNotes(false);
    };
    window.addEventListener("pointerdown", onPointer);
    return () => window.removeEventListener("pointerdown", onPointer);
  }, [menu, notes]);

  return (
    <header data-topbar className="border-border bg-background flex h-[var(--topbar-h)] shrink-0 items-center gap-2 border-b px-3">
      <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-1">
        <Link to={sectionHref} className="text-small text-muted-foreground hover:text-foreground truncate">
          {focused.crumb[0]}
        </Link>
        <span className="text-muted-foreground" aria-hidden>
          /
        </span>
        <Link to={hrefFor(focused.id)} className="text-small truncate">
          {focused.crumb[1]}
        </Link>
      </nav>
      <button
        type="button"
        data-open-palette
        className="border-border text-muted-foreground hover:bg-muted mx-auto flex h-[var(--control-h)] min-w-0 max-w-md flex-1 items-center gap-2 rounded-md border px-2"
        onClick={() => setPaletteOpen(true)}
      >
        <Search className="size-3.5 shrink-0" />
        <span className="text-small hidden truncate sm:inline">Search or run a command…</span>
        <kbd className="text-micro ml-auto hidden md:inline">{mod}K</kbd>
      </button>
      <div ref={menuRef} className="ml-auto flex items-center gap-1">
        <span
          className="text-small text-muted-foreground hidden items-center gap-1 lg:flex"
          title={health.isError ? "Health stream detached." : "Health stream"}
        >
          <span
            className={cn(
              "inline-block size-1.5 rounded-pill",
              health.isError ? "bg-red-500" : health.data ? "bg-green-500" : "bg-slate-400",
            )}
            aria-hidden
          />
          {health.isError ? "Detached" : health.data ? "Live" : "…"}
        </span>
        <div className="relative">
          <IconButton label="Notifications" pressed={notes} onClick={() => setNotes((open) => !open)}>
            <Bell className="size-3.5" />
          </IconButton>
          {notes ? (
            <div data-notifications className="border-border bg-popover elevation-2 absolute right-0 z-20 mt-1 w-72 rounded-lg border p-3">
              <p className="text-h3">Notifications</p>
              <NotificationDigest />
            </div>
          ) : null}
        </div>
        <IconButton
          label={theme === "dark" ? "Light theme" : "Dark theme"}
          pressed={theme === "dark"}
          onClick={toggleTheme}
        >
          {theme === "dark" ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}
        </IconButton>
        <button
          type="button"
          className="text-small hover:bg-muted hidden h-[var(--control-h)] rounded-md px-2 sm:inline"
          aria-pressed={density === "compact"}
          onClick={toggleDensity}
        >
          {density === "compact" ? "Compact" : "Comfortable"}
        </button>
        <div className="relative">
          <button
            type="button"
            className="text-small hover:bg-muted h-[var(--control-h)] rounded-md px-2"
            aria-expanded={menu}
            aria-haspopup="menu"
            onClick={() => setMenu((open) => !open)}
          >
            View
          </button>
          {menu ? (
            <div role="menu" className="border-border bg-popover elevation-2 absolute right-0 z-20 mt-1 w-44 rounded-lg border p-1">
              <MenuItem
                label="Reset layout"
                onSelect={() => {
                  resetLayout();
                  setMenu(false);
                }}
              />
              <MenuItem
                label="Swap panes"
                onSelect={() => {
                  toggleSwap();
                  setMenu(false);
                }}
              />
              <MenuItem
                label="Shortcuts"
                onSelect={() => {
                  setShortcutsOpen(true);
                  setMenu(false);
                }}
              />
            </div>
          ) : null}
        </div>
        <span className="text-small text-muted-foreground hidden px-1 xl:inline" title="This console is single-user.">
          Local operator
        </span>
      </div>
    </header>
  );
}

function NotificationDigest() {
  const health = useHealth();
  const notes = (health.data?.kernel.services ?? []).filter((service) => service.state !== "ok");
  if (!health.data) {
    return <p className="text-small text-muted-foreground mt-1">Waiting for GET /api/health.</p>;
  }
  if (!notes.length) {
    return <p className="text-small text-muted-foreground mt-1">No notifications.</p>;
  }
  return (
    <ul className="mt-2 space-y-2">
      {notes.map((service) => (
        <li key={service.name} data-notification={service.name}>
          <p className="text-small font-medium">
            {service.name} {service.state}
          </p>
          {service.reason ? <p className="text-small text-muted-foreground">{service.reason}</p> : null}
        </li>
      ))}
    </ul>
  );
}

function IconButton({
  label,
  pressed,
  onClick,
  children,
}: {
  label: string;
  pressed?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      className="text-muted-foreground hover:bg-muted hover:text-foreground grid size-7 place-items-center rounded-md"
      onClick={onClick}
    >
      {children}
    </button>
  );
}

function MenuItem({ label, onSelect }: { label: string; onSelect: () => void }) {
  return (
    <button
      type="button"
      role="menuitem"
      className="text-small hover:bg-muted h-8 w-full rounded-sm px-2 text-left"
      onClick={onSelect}
    >
      {label}
    </button>
  );
}
