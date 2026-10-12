import type { ReactNode } from "react";

import { Button } from "@/components/ui/actions.tsx";
import { Card, CardBody, CardFooter, CardHeader, CardTitle } from "@/components/ui/layout.tsx";
import { Badge } from "@/components/ui/display.tsx";
import { ErrorState } from "@/components/ui/feedback.tsx";
import { Skeleton } from "@/components/ui/display.tsx";
import { IconChip, type IconChipKind } from "@/components/kit/icon-chip.tsx";
import { cn } from "@/lib/utils.ts";

export type CardState = "default" | "loading" | "empty" | "error" | "stale" | "live";

export function CardShell({
  title,
  eyebrow,
  kind,
  state = "default",
  children,
  footer,
}: {
  title: string;
  eyebrow?: string;
  kind?: IconChipKind;
  state?: CardState;
  children?: ReactNode;
  footer?: ReactNode;
}) {
  return (
    <Card className="relative overflow-hidden">
      {state === "stale" ? <span className="micro-label absolute top-0 right-0 bg-amber-200 px-1 text-amber-950">Stale</span> : null}
      <CardHeader tint>
        {kind ? <IconChip kind={kind} size={16} spin={state === "live"} /> : null}
        <CardTitle eyebrow={eyebrow}>{title}</CardTitle>
        {state === "live" ? <span className="bg-cyan-500 mm-pulse ml-auto size-1.5 rounded-pill" role="img" aria-label="Live" /> : null}
      </CardHeader>
      <CardBody>
        {state === "loading" ? (
          <div className="flex flex-col gap-2" aria-busy>
            <Skeleton className="h-3 w-2/3" />
            <Skeleton className="h-3 w-full" />
          </div>
        ) : null}
        {state === "empty" ? <p className="text-small text-muted-foreground">Nothing here yet.</p> : null}
        {state === "error" ? <ErrorState message="The card could not load" reason="The kernel returned no body." /> : null}
        {state === "default" || state === "stale" || state === "live" ? children : null}
      </CardBody>
      {footer ? <CardFooter>{footer}</CardFooter> : null}
    </Card>
  );
}

export function StatCard({ state = "default", value = "128", delta = "+4.2%" }: { state?: CardState; value?: string; delta?: string }) {
  return (
    <CardShell title="Events" eyebrow="Kernel" kind="log" state={state} footer={<span className="text-small text-muted-foreground">Since boot</span>}>
      <p className="font-mono text-[20px] leading-6">{value}</p>
      <Badge tone="green">{delta}</Badge>
    </CardShell>
  );
}

export function MeterCard({ state = "default" }: { state?: CardState }) {
  return (
    <CardShell title="Change budget" eyebrow="Budgets" kind="budget" state={state}>
      <meter className="w-full" value={0.42} min={0} max={1} aria-label="Change budget used">
        42%
      </meter>
      <p className="text-small text-muted-foreground mt-1">42% of the change budget. Floor 30%.</p>
    </CardShell>
  );
}

export function ListCard({ state = "default" }: { state?: CardState }) {
  const rows = [
    ["doctor", "ok"],
    ["replay", "ok"],
  ];
  return (
    <CardShell title="Recent" eyebrow="Work" kind="doc" state={state}>
      <ul>
        {rows.map(([title, value]) => (
          <li key={title} className="flex items-center justify-between py-1">
            <span className="text-body">{title}</span>
            <span className="font-mono text-value">{value}</span>
          </li>
        ))}
      </ul>
    </CardShell>
  );
}

export function FeedCard({ state = "default" }: { state?: CardState }) {
  return (
    <CardShell title="Stages" eyebrow="Run" kind="run" state={state}>
      <ol className="flex flex-col gap-1">
        <li className="font-mono text-value">00:01 plan</li>
        <li className="font-mono text-value">00:04 act</li>
      </ol>
    </CardShell>
  );
}

export function ProseCard({ state = "default" }: { state?: CardState }) {
  return (
    <CardShell title="Note" eyebrow="Library" kind="memory" state={state}>
      <p className="text-[14px] leading-[22px]">A claim stays a claim until a promotion writes it down.</p>
    </CardShell>
  );
}

export function CodeCard({ state = "default" }: { state?: CardState }) {
  return (
    <CardShell title="sample.json" eyebrow="JSON" kind="doc" state={state} footer={<Button size="sm" variant="outline">Copy</Button>}>
      <pre className="bg-muted overflow-auto rounded-xs p-2 font-mono text-value">{`{ "ok": true }`}</pre>
    </CardShell>
  );
}

export function VerdictCard({ state = "default" }: { state?: CardState }) {
  return (
    <CardShell title="Firewall" eyebrow="Decision" kind="decision" state={state}>
      <Badge tone="green">Allow</Badge>
      <p className="text-small mt-2">The episode stayed inside the policy.</p>
    </CardShell>
  );
}

export function ProofCard({ state = "default" }: { state?: CardState }) {
  return (
    <CardShell title="Obligation" eyebrow="Verify" kind="proof" state={state}>
      <ol className="list-decimal pl-4 text-small">
        <li>Premise holds.</li>
        <li>Step follows.</li>
      </ol>
      <Badge tone="green">QED</Badge>
    </CardShell>
  );
}

export function ScreenshotCard({ state = "default" }: { state?: CardState }) {
  return (
    <CardShell title="Capture" eyebrow="Audit" kind="chart" state={state}>
      <figure className="border-border rounded-xs border-2 p-1">
        <div className="bg-muted grid h-16 place-items-center" role="img" aria-label="Audit capture">
          <span className="text-small text-muted-foreground">Audit capture</span>
        </div>
        <figcaption className="text-small text-muted-foreground">Promotion gate</figcaption>
      </figure>
    </CardShell>
  );
}

export function cardStateClass(state: CardState) {
  return cn(state === "stale" && "opacity-90");
}
