import { useEffect, useId, useRef, type ReactNode } from "react";

import { Button } from "@/components/ui/actions.tsx";
import { cn } from "@/lib/utils.ts";
import { recipes } from "@/styles/recipes.ts";

export function Dialog({
  open,
  title,
  children,
  onClose,
  inline = false,
}: {
  open: boolean;
  title: string;
  children?: ReactNode;
  onClose?: () => void;
  inline?: boolean;
}) {
  const titleId = useId();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open || inline) return;
    const previous = document.activeElement;
    ref.current?.focus();
    return () => {
      if (previous instanceof HTMLElement) previous.focus();
    };
  }, [inline, open]);
  if (!open) return null;
  return (
    <div className={inline ? "relative" : "fixed inset-0 z-50 grid place-items-center p-4"} data-token="dialog">
      {inline ? null : <button type="button" className={cn("absolute inset-0", recipes.scrim)} aria-label="Close dialog" onClick={onClose} />}
      <div
        ref={ref}
        role="dialog"
        aria-modal={inline ? undefined : true}
        aria-labelledby={titleId}
        tabIndex={-1}
        className={cn(recipes.overlay, "relative z-10 w-full max-w-md p-4 outline-none")}
      >
        <h2 id={titleId} className="text-h2">
          {title}
        </h2>
        <div className="mt-3">{children}</div>
      </div>
    </div>
  );
}

export function AlertDialog({
  name,
  idValue,
  typed,
  onTyped,
  onConfirm,
}: {
  name: string;
  idValue: string;
  typed: string;
  onTyped: (value: string) => void;
  onConfirm?: () => void;
}) {
  const titleId = useId();
  const ready = typed === idValue;
  return (
    <div role="alertdialog" aria-labelledby={titleId} data-token="alert-dialog" className={cn(recipes.overlay, "max-w-md p-4")}>
      <h2 id={titleId} className="text-h2">
        Apply {name}
      </h2>
      <p className="text-body mt-2">
        Type <span className="font-mono">{idValue}</span> to confirm. This cannot be undone.
      </p>
      <input
        aria-label={`Type ${idValue} to confirm`}
        className={cn(recipes.control, "mt-3 h-[var(--control-h)] w-full px-2 font-mono")}
        value={typed}
        onChange={(event) => onTyped(event.target.value)}
      />
      <div className="mt-3 flex justify-end gap-2">
        <Button variant="outline">Cancel</Button>
        <Button variant="destructive" disabled={!ready} onClick={onConfirm}>
          Apply
        </Button>
      </div>
    </div>
  );
}

export function Popover({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div data-token="popover" className={cn(recipes.overlay, "elevation-1 w-64 p-3")}>
      <p className="text-h3">{title}</p>
      <div className="mt-2">{children}</div>
    </div>
  );
}

export function Sheet({ title, children }: { title: string; children: ReactNode }) {
  return (
    <aside data-token="sheet" aria-label={title} className={cn(recipes.overlay, "flex h-64 w-full max-w-sm flex-col p-4")}>
      <h2 className="text-h2">{title}</h2>
      <div className="mt-3 min-h-0 flex-1 overflow-auto">{children}</div>
    </aside>
  );
}

export function Drawer({ title, children }: { title: string; children: ReactNode }) {
  return (
    <aside data-token="drawer" aria-label={title} className={cn(recipes.overlay, "w-full max-w-[480px] p-4")}>
      <h2 className="text-h3">{title}</h2>
      <div className="mt-2">{children}</div>
    </aside>
  );
}

export function MenuList({ label, items }: { label: string; items: { id: string; label: string; disabled?: boolean }[] }) {
  const shown = items.length > 0 ? items : [{ id: "none", label: "No actions", disabled: true }];
  return (
    <div role="menu" aria-label={label} data-token="menu" className={cn(recipes.overlay, "elevation-1 w-48 p-1")}>
      {shown.map((item) => (
        <button key={item.id} type="button" role="menuitem" disabled={item.disabled} className="hover:bg-muted h-8 w-full rounded-sm px-2 text-left text-small disabled:opacity-50">
          {item.label}
        </button>
      ))}
    </div>
  );
}

export function DropdownMenu(props: { label: string; items: { id: string; label: string; disabled?: boolean }[] }) {
  return (
    <div data-token="dropdown-menu">
      <MenuList {...props} />
    </div>
  );
}

export function ContextMenu(props: { label: string; items: { id: string; label: string; disabled?: boolean }[] }) {
  return (
    <div data-token="context-menu">
      <MenuList {...props} />
    </div>
  );
}

export function Lightbox({ alt, caption }: { alt: string; caption: string }) {
  return (
    <figure data-token="lightbox" className={cn(recipes.overlay, "p-2")}>
      <div className="bg-muted grid h-32 place-items-center rounded-xs" role="img" aria-label={alt}>
        <span className="text-small text-muted-foreground">{alt}</span>
      </div>
      <figcaption className="text-small text-muted-foreground mt-2">{caption}</figcaption>
    </figure>
  );
}
