import { useHealth } from "@/lib/api/health.ts";
import type { HealthEnvelope, ServiceState } from "@/lib/wire/index.ts";

export function StatusBar() {
  const health = useHealth();
  const kernel = health.data?.kernel;
  const state: ServiceState | "unreachable" | "loading" = health.isError ? "unreachable" : (kernel?.state ?? "loading");
  const serviceTitle = kernel
    ? kernel.services
        .filter((service) => service.state !== "ok")
        .map((service) => `${service.name}: ${service.state}${service.reason ? ` — ${service.reason}` : ""}`)
        .join("\n") || "Kernel is serving."
    : "Waiting for GET /api/health.";
  const dir = health.data?.dataDir.split(/[/\\]/).filter(Boolean).pop() ?? "…";

  return (
    <footer
      data-statusbar
      data-kernel-state={state}
      className="border-border bg-status text-muted-foreground flex h-[var(--statusbar-h)] shrink-0 items-center gap-3 overflow-x-auto border-t px-2 font-mono text-[11px] whitespace-nowrap"
    >
      <span title={serviceTitle}>{kernelLabel(state, kernel?.reason)}</span>
      <span>events {health.data ? health.data.eventHead.seq : "…"}</span>
      <span title={llmTitle(health.data)}>{llmLabel(health.data)}</span>
      <span className="ml-auto" title={health.data?.dataDir}>
        {dir}
      </span>
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
