import { useEffect, useState, type ReactNode } from "react";

import { PageFrame } from "@/routes/frame.tsx";
import {
  WIRE_VERSION,
  type HealthEnvelope,
  type ServiceState,
} from "@/lib/wire/index.ts";

const HUES: { name: string; means: string }[] = [
  { name: "azure", means: "Primary actions, selection, focus" },
  { name: "teal", means: "Memory, learning, procedures" },
  { name: "violet", means: "LLM turns, prompts, providers" },
  { name: "amber", means: "Repair, assumptions, budgets" },
  { name: "red", means: "Failures, denials, rollbacks" },
  { name: "green", means: "Verified, promoted, healthy" },
  { name: "cyan", means: "Charts, datasets, live data" },
  { name: "indigo", means: "Tools, actions, sandbox" },
  { name: "rose", means: "Identity, goals, commitments" },
  { name: "slate", means: "Structure, unknown, disabled" },
];

const STEPS = [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950] as const;

const RADII: { token: string; use: string; cls: string }[] = [
  { token: "xs · 2px", use: "Surfaces", cls: "rounded-xs" },
  { token: "sm · 3px", use: "Small controls", cls: "rounded-sm" },
  { token: "md · 4px", use: "Buttons, inputs", cls: "rounded-md" },
  { token: "lg · 6px", use: "Dialogs, drawers", cls: "rounded-lg" },
  { token: "bubble · 8px", use: "Chat bubbles only", cls: "rounded-bubble" },
  { token: "pill", use: "Chips and badges", cls: "rounded-pill" },
];

const NEUTRALS: { name: string; cls: string }[] = [
  { name: "background", cls: "bg-background" },
  { name: "card", cls: "bg-card" },
  { name: "muted", cls: "bg-muted" },
  { name: "sidebar", cls: "bg-sidebar" },
  { name: "border", cls: "bg-border" },
  { name: "primary", cls: "bg-primary" },
  { name: "destructive", cls: "bg-destructive" },
];

type HealthView =
  | { kind: "loading" }
  | { kind: "ok"; health: HealthEnvelope }
  | { kind: "unavailable"; reason: string };

export function TokenGallery() {
  return (
    <PageFrame
      eyebrow="Metamind Console · Milestone 0"
      title="Structured Color"
      lede="Token gallery for the console. Each hue has one meaning. Corner radii stay small."
    >
      <Section title="Neutrals" hint="Structure is drawn with borders and tints.">
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4 lg:grid-cols-7">
          {NEUTRALS.map((item) => (
            <div key={item.name} className="border-border rounded-xs border p-2">
              <div className={`${item.cls} border-border h-10 rounded-xs border`} />
              <p className="micro-label text-muted-foreground mt-2">{item.name}</p>
            </div>
          ))}
        </div>
      </Section>

      <Section title="Ten hues" hint="Step 500 is the core. Use a tint, never a gradient, on a surface.">
        <div className="flex flex-col gap-3">
          {HUES.map((hue) => (
            <div key={hue.name} className="grid grid-cols-[9rem_1fr] items-center gap-3">
              <div>
                <p className="micro-label">{hue.name}</p>
                <p className="text-small text-muted-foreground">{hue.means}</p>
              </div>
              <div className="flex min-w-0">
                {STEPS.map((step) => (
                  <div
                    key={step}
                    title={`${hue.name}-${step}`}
                    className="h-8 flex-1"
                    style={{ backgroundColor: `var(--hue-${hue.name}-${step})` }}
                  />
                ))}
              </div>
            </div>
          ))}
        </div>
      </Section>

      <Section title="Radius" hint="Square-ish surfaces, rounded controls, one soft bubble, pills only for chips.">
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
          {RADII.map((item) => (
            <div key={item.token} className="border-border rounded-xs border p-3">
              <div className={`bg-azure-100 border-azure-200 h-12 border ${item.cls}`} />
              <p className="micro-label mt-2">{item.token}</p>
              <p className="text-small text-muted-foreground">{item.use}</p>
            </div>
          ))}
        </div>
      </Section>

      <Section title="Type" hint="Geist for the UI. Geist Mono for every value that is a value.">
        <div className="border-border rounded-xs grid gap-3 border p-4">
          <p className="text-display">Display</p>
          <p className="text-h1">Document title</p>
          <p className="text-h2">Section header</p>
          <p className="text-body">Body copy is 13px. This is the console baseline.</p>
          <p className="micro-label text-muted-foreground">Micro label</p>
          <p className="text-value font-mono tabular-nums">01H8…9K2 · 1,024 · 4m</p>
        </div>
      </Section>

      <Section title="Motion" hint="120–240ms. Reduced motion zeroes the durations.">
        <div className="flex items-center gap-3">
          <span className="bg-cyan-500 mm-pulse inline-block size-1.5 rounded-pill" />
          <p className="text-small">
            Live signal. Fast {`var(--t-fast)`} is 120ms, base 180ms, slow 240ms.
          </p>
        </div>
      </Section>

      <Section title="Kernel" hint={`Wire version ${WIRE_VERSION}. Unavailable is shown with its reason.`}>
        <HealthPanel />
      </Section>
    </PageFrame>
  );
}

function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint: string;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3">
      <div>
        <h2 className="text-h2">{title}</h2>
        <p className="text-small text-muted-foreground">{hint}</p>
      </div>
      {children}
    </section>
  );
}

function HealthPanel() {
  const view = useHealth();
  if (view.kind === "loading") {
    return <p className="text-small text-muted-foreground">Checking /api/health…</p>;
  }
  if (view.kind === "unavailable") {
    return (
      <div className="border-border bg-card rounded-xs border border-l-[3px] border-l-slate-500 p-4">
        <p className="micro-label text-slate-700 dark:text-slate-300">Unavailable</p>
        <p className="text-body mt-1">{view.reason}</p>
      </div>
    );
  }
  const { health } = view;
  return (
    <div className="border-border bg-card rounded-xs border p-4">
      <div className="flex flex-wrap items-baseline gap-x-6 gap-y-2">
        <Value label="wire" value={String(health.wireVersion)} />
        <Value label="kernel" value={health.kernel.state} />
        <Value label="events" value={String(health.eventHead.seq)} />
        <Value label="data" value={health.dataDir} />
      </div>
      {health.kernel.reason ? (
        <p className="text-small text-muted-foreground mt-3">{health.kernel.reason}</p>
      ) : null}
      <ul className="mt-4 flex flex-col gap-2">
        {health.kernel.services.map((service) => (
          <li key={service.name} className="flex items-start gap-3">
            <StateChip state={service.state} />
            <div>
              <p className="font-mono text-value">{service.name}</p>
              {service.reason ? (
                <p className="text-small text-muted-foreground">{service.reason}</p>
              ) : null}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Value({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="micro-label text-muted-foreground">{label}</p>
      <p className="font-mono text-value tabular-nums">{value}</p>
    </div>
  );
}

function StateChip({ state }: { state: ServiceState }) {
  const tone =
    state === "ok"
      ? "bg-green-50 text-green-700 border-green-200"
      : state === "degraded"
        ? "bg-amber-50 text-amber-700 border-amber-200"
        : "bg-slate-100 text-slate-700 border-slate-200";
  return (
    <span className={`micro-label rounded-pill border px-2 py-0.5 ${tone}`}>{state}</span>
  );
}

function useHealth(): HealthView {
  const [view, setView] = useState<HealthView>({ kind: "loading" });
  useEffect(() => {
    const ctrl = new AbortController();
    fetch("/api/health", { signal: ctrl.signal })
      .then(async (res) => {
        if (!res.ok) {
          const body = (await res.json().catch(() => null)) as {
            reason?: string;
            message?: string;
          } | null;
          const reason = body?.reason || body?.message || `HTTP ${res.status}`;
          setView({ kind: "unavailable", reason });
          return;
        }
        const health = (await res.json()) as HealthEnvelope;
        setView({ kind: "ok", health });
      })
      .catch((err: unknown) => {
        if (ctrl.signal.aborted) {
          return;
        }
        const detail = err instanceof Error ? err.message : "request failed";
        setView({
          kind: "unavailable",
          reason: `mm-serve is not reachable (${detail})`,
        });
      });
    return () => ctrl.abort();
  }, []);
  return view;
}
