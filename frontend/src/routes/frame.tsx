import { useEffect, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";

import { cn } from "@/lib/utils.ts";

const THEME_KEY = "mm-theme";

function readTheme(): "light" | "dark" {
  if (document.documentElement.classList.contains("dark")) {
    return "dark";
  }
  return "light";
}

export function PageFrame({
  eyebrow,
  title,
  lede,
  children,
}: {
  eyebrow: string;
  title: string;
  lede: string;
  children: ReactNode;
}) {
  const [theme, setTheme] = useState<"light" | "dark">(readTheme);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    localStorage.setItem(THEME_KEY, theme);
  }, [theme]);

  return (
    <div className="min-h-full bg-background text-foreground">
      <header className="border-b border-border bg-sidebar">
        <div className="flex items-end justify-between gap-6 px-6 py-4">
          <div>
            <p className="micro-label text-muted-foreground">{eyebrow}</p>
            <h1 className="text-h1 mt-1">{title}</h1>
            <p className="text-small mt-1 max-w-xl text-muted-foreground">{lede}</p>
          </div>
          <div className="flex items-center gap-2">
            <nav className="flex items-center gap-1">
              <FrameLink to="/">Tokens</FrameLink>
              <FrameLink to="/dev/catalog">Catalog</FrameLink>
              <FrameLink to="/workspace">Workspace</FrameLink>
            </nav>
            <button
              type="button"
              className="border-border-strong bg-card text-small h-[var(--control-h)] rounded-md border px-3"
              aria-pressed={theme === "dark"}
              onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
            >
              {theme === "dark" ? "Light theme" : "Dark theme"}
            </button>
          </div>
        </div>
      </header>
      <main className="flex flex-col gap-8 px-6 py-6">{children}</main>
    </div>
  );
}

function FrameLink({ to, children }: { to: string; children: string }) {
  return (
    <Link
      to={to}
      className={cn(
        "text-small text-foreground hover:bg-muted h-[var(--control-h)] rounded-md px-3 leading-[var(--control-h)]",
      )}
    >
      {children}
    </Link>
  );
}
