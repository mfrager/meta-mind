import type { ReactNode } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";

import { cn } from "@/lib/utils.ts";
import { recipes } from "@/styles/recipes.ts";

export function Card({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <section data-token="card" className={cn(recipes.surface, className)}>
      {children}
    </section>
  );
}

export function CardHeader({ children, tint = false }: { children: ReactNode; tint?: boolean }) {
  return <header className={cn("flex items-center gap-2 border-b border-border px-3 py-2", tint && recipes.headerTint)}>{children}</header>;
}

export function CardTitle({ eyebrow, children }: { eyebrow?: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      {eyebrow ? <p className="micro-label text-muted-foreground">{eyebrow}</p> : null}
      <h3 className="text-h3 truncate">{children}</h3>
    </div>
  );
}

export function CardBody({ children }: { children: ReactNode }) {
  return <div className="p-3">{children}</div>;
}

export function CardFooter({ children }: { children: ReactNode }) {
  return <footer className="border-border flex items-center justify-between gap-2 border-t px-3 py-2">{children}</footer>;
}

export function Accordion({
  items,
  openId,
  variant = "panel",
}: {
  items: { id: string; title: string; body: string; meta?: string }[];
  openId?: string;
  variant?: "panel" | "flush" | "faq";
}) {
  return (
    <div data-token={`accordion ${variant}`} className={variant === "panel" ? "flex flex-col gap-2" : "border-border border-y"}>
      {items.map((item) => (
        <details key={item.id} open={item.id === openId} className={variant === "panel" ? cn(recipes.surface, "px-3") : "border-border border-b last:border-b-0"}>
          <summary className="flex h-8 cursor-pointer list-none items-center justify-between text-small">
            <span className={variant === "faq" ? "text-h3" : undefined}>{item.title}</span>
            {item.meta ? <span className="micro-label text-muted-foreground">{item.meta}</span> : null}
          </summary>
          <p className="text-small text-muted-foreground pb-2">{item.body}</p>
        </details>
      ))}
    </div>
  );
}

export function Collapsible({ title, children, open = false }: { title: string; children: ReactNode; open?: boolean }) {
  return (
    <details data-token="collapsible" open={open} className={cn(recipes.surface, "px-3")}>
      <summary className="flex h-8 cursor-pointer list-none items-center text-small">{title}</summary>
      <div className="pb-2">{children}</div>
    </details>
  );
}

export function Container({ children }: { children: ReactNode }) {
  return (
    <div data-token="container" className="mx-auto w-full max-w-5xl">
      {children}
    </div>
  );
}

export function AspectRatio({ ratio = "16 / 9", children }: { ratio?: string; children?: ReactNode }) {
  return (
    <div data-token="aspect-ratio" className="bg-muted grid place-items-center rounded-xs" style={{ aspectRatio: ratio }}>
      {children}
    </div>
  );
}

export function SplitPane({ left, right }: { left: ReactNode; right: ReactNode }) {
  return (
    <div data-token="split-pane" className="grid min-h-24 grid-cols-2">
      <div className="border-border border-r p-2">{left}</div>
      <div className="p-2">{right}</div>
    </div>
  );
}

export function ResizablePanes() {
  return (
    <div data-token="resizable" className="h-24">
      <Group orientation="horizontal" id="catalog-split">
        <Panel id="left" defaultSize="55%" minSize="25%" maxSize="75%">
          <div className="h-full p-2 text-small">Document</div>
        </Panel>
        <Separator className="pane-separator" />
        <Panel id="right" defaultSize="45%" minSize="25%" maxSize="75%">
          <div className="h-full p-2 text-small">Chat</div>
        </Panel>
      </Group>
    </div>
  );
}
