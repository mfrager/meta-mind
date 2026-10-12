import { cva, type VariantProps } from "class-variance-authority";
import { Loader2 } from "lucide-react";
import type { ButtonHTMLAttributes, ReactNode } from "react";

import { cn } from "@/lib/utils.ts";
import { focusPreview, type Preview } from "@/components/ui/types.ts";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 rounded-md font-medium transition-colors duration-[var(--t-fast)] outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 motion-reduce:transition-none",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground",
        outline: "border-border bg-card border",
        ghost: "bg-transparent",
        destructive: "bg-destructive text-destructive-foreground",
        link: "text-primary underline-offset-2",
      },
      size: {
        xs: "h-6 px-2 text-micro",
        sm: "h-7 px-2 text-small",
        md: "h-[var(--control-h)] px-3 text-body",
      },
      preview: {
        none: "",
        hover: "",
        active: "",
        focus: "",
      },
    },
    compoundVariants: [
      { variant: "default", preview: "hover", class: "bg-azure-600" },
      { variant: "default", preview: "active", class: "bg-azure-700" },
      { variant: "outline", preview: "hover", class: "bg-muted" },
      { variant: "outline", preview: "active", class: "bg-accent" },
      { variant: "ghost", preview: "hover", class: "bg-muted" },
      { variant: "ghost", preview: "active", class: "bg-accent" },
      { variant: "destructive", preview: "hover", class: "bg-red-600" },
      { variant: "destructive", preview: "active", class: "bg-red-700" },
      { variant: "link", preview: "hover", class: "underline" },
      { variant: "link", preview: "active", class: "underline" },
    ],
    defaultVariants: { variant: "default", size: "md", preview: "none" },
  },
);

export function Button({
  variant,
  size,
  preview = "none",
  loading = false,
  className,
  children,
  disabled,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> &
  VariantProps<typeof buttonVariants> & { loading?: boolean; preview?: Preview }) {
  return (
    <button
      type="button"
      data-token={`button ${variant ?? "default"} ${size ?? "md"}`}
      className={cn(buttonVariants({ variant, size, preview }), focusPreview(preview), className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...props}
    >
      {loading ? <Loader2 className="size-3.5 motion-safe:animate-spin" aria-hidden /> : null}
      {children}
    </button>
  );
}

export function Toggle({
  pressed,
  preview,
  children,
  className,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { pressed: boolean; preview?: Preview }) {
  return (
    <button
      type="button"
      aria-pressed={pressed}
      data-token="toggle"
      className={cn(
        "border-border h-[var(--control-h)] rounded-md border px-3 text-small",
        pressed || preview === "active" ? "bg-accent" : "bg-card",
        preview === "hover" && "bg-muted",
        focusPreview(preview),
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}

export function ToggleGroup({
  value,
  options,
  onChange,
  label,
}: {
  value: string;
  options: string[];
  onChange?: (value: string) => void;
  label: string;
}) {
  return (
    <div role="group" aria-label={label} data-token="toggle-group" className="border-border flex overflow-hidden rounded-md border">
      {options.map((option) => (
        <button
          key={option}
          type="button"
          aria-pressed={value === option}
          className={cn("h-7 px-2 text-small", value === option ? "bg-accent" : "hover:bg-muted")}
          onClick={() => onChange?.(option)}
        >
          {option}
        </button>
      ))}
    </div>
  );
}

export function ButtonGroup({ children, label }: { children: ReactNode; label: string }) {
  return (
    <div role="group" aria-label={label} data-token="button-group" className="border-border flex overflow-hidden rounded-md border">
      {children}
    </div>
  );
}
