import { useEffect, useState } from "react";
import { Link, Outlet, useLocation } from "react-router-dom";
import {
  Activity,
  Brain,
  Boxes,
  FilePen,
  FlaskConical,
  Gauge,
  GitCompare,
  Info,
  LayoutDashboard,
  Library,
  MessageSquare,
  Moon,
  Network,
  PanelLeft,
  PenLine,
  Pin,
  Scale,
  ScrollText,
  Settings,
  Shield,
  Stethoscope,
  Sun,
  SwatchBook,
  UserRound,
  Wrench,
  type LucideIcon,
} from "lucide-react";

import { CommandPalette } from "@/components/shell/command-palette.tsx";
import { ToastHost } from "@/components/ui/sonner.tsx";
import { KeyboardMap } from "@/components/shell/keyboard-map.tsx";
import { ShortcutDialog } from "@/components/shell/shortcut-dialog.tsx";
import { StatusBar } from "@/components/shell/statusbar.tsx";
import { TopBar } from "@/components/shell/topbar.tsx";
import { useHealth } from "@/lib/api/health.ts";
import { DOCUMENTS, NAV_SECTIONS, hrefFor } from "@/lib/nav.ts";
import { usePaneStore } from "@/lib/store/pane.ts";
import { usePrefsStore } from "@/lib/store/prefs.ts";
import { TONE_CHIP } from "@/lib/tone.ts";
import { useViewportTier, type Tier } from "@/lib/use-tier.ts";
import { WIRE_VERSION } from "@/lib/wire/index.ts";
import { cn } from "@/lib/utils.ts";

const ICONS: Record<string, LucideIcon> = {
  overview: LayoutDashboard,
  workspace: MessageSquare,
  pinned: Pin,
  drafts: FilePen,
  being: UserRound,
  memory: Brain,
  epistemic: Scale,
  library: Library,
  ontology: Network,
  changesets: GitCompare,
  modules: Boxes,
  policies: Shield,
  pi: PenLine,
  regressions: FlaskConical,
  runs: Activity,
  logs: ScrollText,
  provenance: Network,
  tools: Wrench,
  budgets: Gauge,
  doctor: Stethoscope,
  settings: Settings,
  catalog: SwatchBook,
  about: Info,
};

export function AppShell() {
  const ready = useShellHydrated();
  const tier = useViewportTier();
  const collapsed = usePaneStore((state) => state.sidebarCollapsed);
  const sheet = usePaneStore((state) => state.sidebarSheet);
  const setSheet = usePaneStore((state) => state.setSheet);
  const inline = tier === "wide" || tier === "medium";
  const rail = tier === "medium" || (tier === "wide" && collapsed);
  const sidebarMode = inline ? (rail ? "rail" : "expanded") : "hidden";

  useEffect(() => {
    if (tier === "wide") setSheet(false);
  }, [setSheet, tier]);

  if (!ready) {
    return <div className="bg-background h-full" data-shell-pending />;
  }

  return (
    <div className="bg-background text-foreground flex h-full flex-col" data-shell data-tier={tier} data-sidebar={sidebarMode}>
      <div className="flex min-h-0 flex-1">
        {inline ? <Sidebar tier={tier} rail={rail} /> : null}
        {sheet && tier !== "wide" ? (
          <>
            <button type="button" className="mm-overlay" aria-label="Close menu" onClick={() => setSheet(false)} />
            <div className="fixed inset-y-0 left-0 z-40 flex" data-sidebar-sheet>
              <Sidebar tier={tier} rail={false} onNavigate={() => setSheet(false)} />
            </div>
          </>
        ) : null}
        <div className="flex min-w-0 flex-1 flex-col">
          <TopBar />
          <div className="min-h-0 flex-1 overflow-hidden">
            <Outlet />
          </div>
        </div>
      </div>
      <StatusBar />
      <CommandPalette />
      <ShortcutDialog />
      <KeyboardMap />
      <ToastHost />
      <SyncPrefs />
    </div>
  );
}

function Sidebar({
  tier,
  rail,
  onNavigate,
}: {
  tier: Tier;
  rail: boolean;
  onNavigate?: () => void;
}) {
  const { pathname, search } = useLocation();
  const doc = new URLSearchParams(search).get("doc");
  const toggleSidebar = usePaneStore((state) => state.toggleSidebar);
  const setSheet = usePaneStore((state) => state.setSheet);
  const health = useHealth();
  const theme = usePrefsStore((state) => state.theme);
  const toggleTheme = usePrefsStore((state) => state.toggleTheme);
  const density = usePrefsStore((state) => state.density);
  const toggleDensity = usePrefsStore((state) => state.toggleDensity);
  const kernel = health.data?.kernel.state ?? (health.isError ? "unreachable" : "…");

  const grip = () => {
    if (onNavigate) {
      onNavigate();
      return;
    }
    if (tier === "wide") toggleSidebar();
    else setSheet(true);
  };

  return (
    <nav
      aria-label="Menu"
      data-sidebar-panel={rail ? "rail" : "expanded"}
      className={cn(
        "border-border bg-sidebar text-sidebar-foreground sidebar-width flex h-full shrink-0 flex-col border-r",
        rail ? "w-[var(--sidebar-rail)]" : "w-[var(--sidebar-w)]",
      )}
    >
      <div className="flex h-[var(--topbar-h)] items-center px-3">
        <span className="text-h3 truncate">{rail ? "M" : "Metamind"}</span>
      </div>
      <div className="pane-scroll flex min-h-0 flex-1 flex-col gap-3 overflow-auto px-2 pb-2">
        {NAV_SECTIONS.map((section) => (
          <div key={section.section} className="flex flex-col">
            {rail ? null : <p className="micro-label text-muted-foreground px-2 py-1">{section.section}</p>}
            {section.ids.map((id) => {
              const meta = DOCUMENTS[id];
              const Icon = ICONS[id] ?? Info;
              const current = isCurrent(id, pathname, doc);
              return (
                <Link
                  key={id}
                  to={hrefFor(id)}
                  aria-current={current ? "page" : undefined}
                  title={meta.title}
                  className={cn(
                    "nav-row rounded-none border-l-2",
                    current ? "border-azure-500 bg-accent" : "border-transparent hover:bg-muted",
                    rail ? "justify-center px-0" : "gap-2 px-2",
                  )}
                  onClick={onNavigate}
                >
                  <span className={cn("grid size-5 shrink-0 place-items-center rounded-xs", TONE_CHIP[meta.tone])}>
                    <Icon className="size-3.5" aria-hidden />
                  </span>
                  {rail ? (
                    <span className="sr-only">{meta.title}</span>
                  ) : (
                    <span className="truncate">{meta.title}</span>
                  )}
                  {!rail && id === "about" ? (
                    <span className="micro-label text-muted-foreground ml-auto">v{WIRE_VERSION}</span>
                  ) : null}
                </Link>
              );
            })}
          </div>
        ))}
      </div>
      <div className="border-border flex flex-col gap-1 border-t p-2">
        <Link
          to={hrefFor("doctor")}
          title={`Kernel ${kernel}`}
          className={cn("nav-row hover:bg-muted border-l-2 border-transparent", rail ? "justify-center px-0" : "gap-2 px-2")}
          onClick={onNavigate}
        >
          <span
            className={cn(
              "inline-block size-1.5 rounded-pill",
              kernel === "ok" ? "bg-green-500" : kernel === "degraded" ? "bg-amber-500" : kernel === "unreachable" ? "bg-red-500" : "bg-slate-400",
            )}
            aria-hidden
          />
          {rail ? <span className="sr-only">Kernel {kernel}</span> : <span className="truncate">Kernel {kernel}</span>}
        </Link>
        <button
          type="button"
          className={cn("nav-row hover:bg-muted border-l-2 border-transparent", rail ? "justify-center px-0" : "gap-2 px-2")}
          aria-label={theme === "dark" ? "Light theme" : "Dark theme"}
          onClick={toggleTheme}
        >
          {theme === "dark" ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}
          {rail ? null : <span>{theme === "dark" ? "Light theme" : "Dark theme"}</span>}
        </button>
        <button
          type="button"
          className={cn("nav-row hover:bg-muted border-l-2 border-transparent", rail ? "justify-center px-0" : "gap-2 px-2")}
          aria-pressed={density === "compact"}
          aria-label={density === "compact" ? "Comfortable density" : "Compact density"}
          onClick={toggleDensity}
        >
          <span className="micro-label">{density === "compact" ? "C" : "D"}</span>
          {rail ? null : <span>{density === "compact" ? "Compact" : "Comfortable"}</span>}
        </button>
        <button
          type="button"
          className={cn("nav-row hover:bg-muted border-l-2 border-transparent", rail ? "justify-center px-0" : "gap-2 px-2")}
          aria-label={onNavigate ? "Close menu" : rail ? "Expand sidebar" : "Collapse sidebar"}
          onClick={grip}
        >
          <PanelLeft className="size-3.5" />
          {rail ? null : <span>{onNavigate ? "Close" : "Collapse"}</span>}
        </button>
      </div>
    </nav>
  );
}

function isCurrent(id: string, pathname: string, doc: string | null): boolean {
  if (id === "catalog") return pathname === "/dev/catalog";
  if (id === "workspace") return pathname.startsWith("/chat");
  if (id === "overview") return pathname === "/" && (!doc || doc === "overview");
  if (pathname.startsWith("/map/")) return pathname.split("/")[2] === id;
  return pathname === "/" && doc === id;
}

function useShellHydrated(): boolean {
  const [ready, setReady] = useState(
    () => usePaneStore.persist.hasHydrated() && usePrefsStore.persist.hasHydrated(),
  );

  useEffect(() => {
    const finish = () => {
      if (usePaneStore.persist.hasHydrated() && usePrefsStore.persist.hasHydrated()) setReady(true);
    };
    finish();
    const stopPane = usePaneStore.persist.onFinishHydration(finish);
    const stopPrefs = usePrefsStore.persist.onFinishHydration(finish);
    return () => {
      stopPane();
      stopPrefs();
    };
  }, []);

  return ready;
}

function SyncPrefs() {
  const theme = usePrefsStore((state) => state.theme);
  const density = usePrefsStore((state) => state.density);
  const [hydrated, setHydrated] = useState(() => usePrefsStore.persist.hasHydrated());

  useEffect(() => {
    if (usePrefsStore.persist.hasHydrated()) setHydrated(true);
    return usePrefsStore.persist.onFinishHydration(() => setHydrated(true));
  }, []);

  useEffect(() => {
    if (!hydrated) return;
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.dataset.density = density;
    localStorage.setItem("mm-theme", theme);
  }, [density, hydrated, theme]);

  return null;
}
