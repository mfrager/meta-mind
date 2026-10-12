import { Command as Cmdk } from "cmdk";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils.ts";

export function Tabs({
  tabs,
  value,
  onChange,
  label,
}: {
  tabs: { id: string; label: string }[];
  value: string;
  onChange?: (id: string) => void;
  label: string;
}) {
  return (
    <div data-token="tabs" role="tablist" aria-label={label} className="border-border flex gap-3 border-b">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={value === tab.id}
          className={cn("text-small -mb-px border-b-2 pb-1", value === tab.id ? "border-azure-500" : "border-transparent")}
          onClick={() => onChange?.(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}

export function Breadcrumb({ items }: { items: { href: string; label: string }[] }) {
  return (
    <nav data-token="breadcrumb" aria-label="Breadcrumb">
      <ol className="flex items-center gap-1 text-small">
        {items.map((item, index) => (
          <li key={item.href} className="flex items-center gap-1">
            {index > 0 ? <span aria-hidden>/</span> : null}
            <a href={item.href} className="hover:underline">
              {item.label}
            </a>
          </li>
        ))}
      </ol>
    </nav>
  );
}

export function Pagination({ page, pages }: { page: number; pages: number }) {
  return (
    <nav data-token="pagination" aria-label="Pagination" className="flex items-center gap-1">
      <button type="button" className="border-border h-7 rounded-md border px-2 text-small" aria-label="Previous page">
        Prev
      </button>
      <span className="font-mono text-value">
        {page} / {pages}
      </span>
      <button type="button" className="border-border h-7 rounded-md border px-2 text-small" aria-label="Next page">
        Next
      </button>
    </nav>
  );
}

export function Menubar({ menus }: { menus: string[] }) {
  return (
    <div data-token="menubar" role="menubar" aria-label="Application" className="border-border flex gap-1 border-b">
      {menus.map((menu) => (
        <button key={menu} type="button" role="menuitem" className="hover:bg-muted h-8 rounded-sm px-2 text-small">
          {menu}
        </button>
      ))}
    </div>
  );
}

export function NavigationMenu({ items }: { items: { href: string; label: string; current?: boolean }[] }) {
  return (
    <nav data-token="navigation-menu" aria-label="Sections" className="flex gap-2">
      {items.map((item) => (
        <a key={item.href} href={item.href} aria-current={item.current ? "page" : undefined} className={cn("text-small", item.current && "border-azure-500 border-b-2")}>
          {item.label}
        </a>
      ))}
    </nav>
  );
}

export function Stepper({ steps, current }: { steps: string[]; current: number }) {
  return (
    <ol data-token="stepper" aria-label="Steps" className="flex gap-2">
      {steps.map((step, index) => (
        <li key={step} className="flex items-center gap-1 text-small" aria-current={index === current ? "step" : undefined}>
          <span className={cn("grid size-5 place-items-center rounded-pill border font-mono text-micro", index <= current ? "border-azure-500 bg-azure-100" : "border-border")}>
            {index + 1}
          </span>
          {step}
        </li>
      ))}
    </ol>
  );
}

export function CommandMenu({ items, label = "Command menu" }: { items: string[]; label?: string }) {
  return (
    <Cmdk label={label} data-token="command" className="border-border rounded-md border">
      <Cmdk.Input placeholder="Search commands…" aria-label="Search commands" />
        <Cmdk.List>
          <Cmdk.Empty>No match</Cmdk.Empty>
          {items.length === 0 ? (
            <Cmdk.Item value="none" disabled>
              No commands
            </Cmdk.Item>
          ) : (
            items.map((item) => (
              <Cmdk.Item key={item} value={item}>
                {item}
              </Cmdk.Item>
            ))
          )}
        </Cmdk.List>
    </Cmdk>
  );
}

export function SidebarFrame({ children, label = "Sample menu" }: { children: ReactNode; label?: string }) {
  return (
    <nav data-token="sidebar" aria-label={label} className="border-border bg-sidebar w-56 rounded-xs border p-2">
      {children}
    </nav>
  );
}
