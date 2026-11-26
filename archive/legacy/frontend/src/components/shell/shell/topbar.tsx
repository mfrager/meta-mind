import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { Moon, Sun } from "lucide-react";

import { useHealth } from "@/lib/api/health.ts";
import { usePrefsStore } from "@/lib/store/prefs.ts";
import { useSessionStore } from "@/lib/store/session.ts";
import { cn } from "@/lib/utils.ts";

export function TopBar() {
  const health = useHealth();
  const theme = usePrefsStore((state) => state.theme);
  const toggleTheme = usePrefsStore((state) => state.toggleTheme);
  const density = usePrefsStore((state) => state.density);
  const toggleDensity = usePrefsStore((state) => state.toggleDensity);
  const title = useSessionStore((state) => state.title);
  const sessionId = useSessionStore((state) => state.id);
  const setTitle = useSessionStore((state) => state.setTitle);
  const [menu, setMenu] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(title);
  const menuRef = useRef<HTMLDivElement>(null);
  const problems = (health.data?.kernel.services ?? []).filter((service) => service.state !== "ok");
  const kernelTitle = problems.length
    ? problems.map((service) => `${service.name}: ${service.state}${service.reason ? ` — ${service.reason}` : ""}`).join("\n")
    : "Kernel is serving.";

  useEffect(() => {
    setDraft(title);
  }, [title]);

  useEffect(() => {
    if (!menu) return;
    const onPointer = (event: PointerEvent) => {
      if (!(event.target instanceof Node)) return;
      if (menuRef.current?.contains(event.target)) return;
      setMenu(false);
    };
    window.addEventListener("pointerdown", onPointer);
    return () => window.removeEventListener("pointerdown", onPointer);
  }, [menu]);

  const saveTitle = async () => {
    const next = draft.trim();
    setEditing(false);
    if (!next || !sessionId || next === title) return;
    const response = await fetch(`/api/chat/sessions/${encodeURIComponent(sessionId)}/title`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: next }),
    });
    if (response.ok) setTitle(next);
  };

  return (
    <header data-topbar className="border-border bg-background flex h-[var(--topbar-h)] shrink-0 items-center gap-2 border-b px-3">
      <Link to="/" className="text-h3 shrink-0">
        Metamind
      </Link>
      {editing ? (
        <input
          aria-label="Session title"
          value={draft}
          className="border-border h-[var(--control-h)] min-w-0 flex-1 rounded-md border px-2 text-small"
          onChange={(event) => setDraft(event.target.value)}
          onBlur={() => void saveTitle()}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void saveTitle();
            }
          }}
        />
      ) : (
        <button
          type="button"
          className="text-small hover:bg-muted h-[var(--control-h)] min-w-0 flex-1 truncate rounded-md px-2 text-left"
          onClick={() => setEditing(true)}
        >
          {title}
        </button>
      )}
      <span className="inline-flex items-center gap-1" title={kernelTitle}>
        <span
          data-kernel-dot
          className={cn(
            "inline-block size-1.5 rounded-pill",
            health.isError ? "bg-red-500" : health.data?.kernel.state === "ok" ? "bg-green-500" : "bg-amber-500",
          )}
          aria-hidden
        />
        <span className="sr-only">Kernel {health.data?.kernel.state ?? "unknown"}</span>
      </span>
      <div ref={menuRef} className="ml-auto flex items-center gap-1">
        <IconButton label={theme === "dark" ? "Light theme" : "Dark theme"} pressed={theme === "dark"} onClick={toggleTheme}>
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
            More
          </button>
          {menu ? (
            <div role="menu" className="border-border bg-popover elevation-2 absolute right-0 z-20 mt-1 w-44 rounded-lg border p-1">
              <Link role="menuitem" className="text-small hover:bg-muted flex h-8 items-center rounded-sm px-2" to="/settings" onClick={() => setMenu(false)}>
                Settings
              </Link>
              <Link role="menuitem" className="text-small hover:bg-muted flex h-8 items-center rounded-sm px-2" to="/dev/catalog" onClick={() => setMenu(false)}>
                Catalog
              </Link>
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
