import { useEffect, useState } from "react";
import { Outlet } from "react-router-dom";

import { KeyboardMap } from "@/components/shell/keyboard-map.tsx";
import { ToastHost } from "@/components/ui/sonner.tsx";
import { ShortcutDialog } from "@/components/shell/shortcut-dialog.tsx";
import { StatusBar } from "@/components/shell/statusbar.tsx";
import { TopBar } from "@/components/shell/topbar.tsx";
import { usePaneStore } from "@/lib/store/pane.ts";
import { usePrefsStore } from "@/lib/store/prefs.ts";
import { useViewportTier } from "@/lib/use-tier.ts";

export function AppShell() {
  const ready = useShellHydrated();
  const tier = useViewportTier();

  if (!ready) {
    return <div className="bg-background h-full" data-shell-pending />;
  }

  return (
    <div className="bg-background text-foreground flex h-full flex-col" data-shell data-tier={tier}>
      <div className="flex min-h-0 flex-1 flex-col">
        <TopBar />
        <div className="min-h-0 flex-1 overflow-hidden">
          <Outlet />
        </div>
      </div>
      <StatusBar />
      <ShortcutDialog />
      <KeyboardMap />
      <ToastHost />
      <SyncPrefs />
    </div>
  );
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
