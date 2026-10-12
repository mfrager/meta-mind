import { useEffect, useState, type ReactNode } from "react";
import { useNavigate } from "react-router-dom";

import { useHealth } from "@/lib/api/health.ts";
import { hrefFor } from "@/lib/nav.ts";
import type { HealthEnvelope, ServiceState } from "@/lib/wire/index.ts";

export function StatusBar() {
  const health = useHealth();
  const navigate = useNavigate();
  const [now, setNow] = useState(() => Date.now());
  const [runReason, setRunReason] = useState("no run is recorded");

  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);

  useEffect(() => {
    let cancelled = false;
    void fetch("/api/runs")
      .then((res) => res.json())
      .then((body: { reason?: string }) => {
        if (!cancelled && body.reason) setRunReason(body.reason);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  const kernel = health.data?.kernel;
  const state: ServiceState | "unreachable" | "loading" = health.isError
    ? "unreachable"
    : (kernel?.state ?? "loading");
  const label = kernelLabel(state, kernel?.reason);
  const serviceTitle = kernel
    ? kernel.services
        .filter((service) => service.state !== "ok")
        .map((service) => `${service.name}: ${service.state}${service.reason ? ` — ${service.reason}` : ""}`)
        .join("\n") || "Kernel is serving."
    : "Waiting for GET /api/health.";

  const age = health.dataUpdatedAt > 0 ? formatAge(now - health.dataUpdatedAt) : null;
  const stream = health.isError ? "stream detached" : age ? `stream ✓ ${age}` : "stream …";
  const clock = new Date(now).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

  return (
    <footer
      data-statusbar
      data-kernel-state={state}
      className="border-border bg-status text-muted-foreground flex h-[var(--statusbar-h)] shrink-0 items-center gap-3 overflow-x-auto border-t px-2 font-mono text-[11px] whitespace-nowrap"
    >
      <BarButton
        label={label}
        title={serviceTitle}
        live
        onClick={() => navigate(hrefFor("doctor"))}
      >
        <Dot state={state} />
        <span>{label}</span>
      </BarButton>
      <BarButton
        label="Event head"
        title="Open logs"
        onClick={() => navigate(hrefFor("logs"))}
      >
        events {health.data ? health.data.eventHead.seq : "…"}
      </BarButton>
      <BarButton
        label="Budgets"
        title="Unavailable — budget meters arrive with self-engineering."
        onClick={() => navigate(hrefFor("budgets"))}
      >
        <span>budgets</span>
        <span className="flex gap-0.5" aria-hidden>
          {Array.from({ length: 4 }, (_, index) => (
            <span key={index} className="bg-muted-foreground/30 inline-block h-2 w-1.5 rounded-xs" />
          ))}
        </span>
      </BarButton>
      <BarButton
        label="Language model"
        title={llmTitle(health.data)}
        onClick={() => navigate(hrefFor("llm"))}
      >
        {llmLabel(health.data)}
      </BarButton>
      <BarButton label="Run" title={runReason} onClick={() => navigate(hrefFor("runs"))}>
        run —
      </BarButton>
      <span className="ml-auto" title={health.isError ? "The health stream is detached." : "Last successful health fetch."}>
        {stream}
      </span>
      <time dateTime={new Date(now).toISOString()}>{clock}</time>
    </footer>
  );
}

function llmService(health: HealthEnvelope | undefined) {
  return health?.kernel.services.find((service) => service.name === "llm");
}

function llmTitle(health: HealthEnvelope | undefined): string {
  return llmService(health)?.reason ?? "no language-model provider is configured";
}

function llmLabel(health: HealthEnvelope | undefined): string {
  const service = llmService(health);
  if (service?.state !== "ok") return "llm unavailable";
  const model = service.reason?.match(/model=([^;]+)/)?.[1];
  return model ? `llm ${model}` : "llm";
}

function kernelLabel(state: ServiceState | "unreachable" | "loading", reason?: string | null): string {
  if (state === "loading") return "kernel …";
  if (state === "unreachable") return "kernel unreachable";
  if (state === "ok") return "kernel healthy";
  return `${state}: ${reason ?? "no reason reported"}`;
}

function formatAge(ms: number): string {
  const seconds = Math.max(0, ms) / 1000;
  if (seconds < 10) return `${seconds.toFixed(1)}s ago`;
  return `${Math.round(seconds)}s ago`;
}

function Dot({ state }: { state: ServiceState | "unreachable" | "loading" }) {
  const color =
    state === "ok"
      ? "bg-green-500"
      : state === "degraded"
        ? "bg-amber-500"
        : state === "unreachable"
          ? "bg-red-500"
          : state === "unavailable"
            ? "bg-slate-500"
            : "bg-slate-400";
  return <span className={`inline-block size-1.5 rounded-pill ${color}`} aria-hidden />;
}

function BarButton({
  label,
  title,
  onClick,
  live,
  children,
}: {
  label: string;
  title: string;
  onClick: () => void;
  live?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={title}
      aria-live={live ? "polite" : undefined}
      className="hover:text-foreground flex items-center gap-1"
      onClick={onClick}
    >
      {children}
    </button>
  );
}
