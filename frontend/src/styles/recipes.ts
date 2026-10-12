/** Shared class recipes. Components consume these instead of inventing colors or radii. */
export const recipes = {
  surface: "border-border bg-card text-card-foreground rounded-xs border",
  control:
    "border-input bg-card text-foreground rounded-md border outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:opacity-50",
  overlay: "border-border bg-popover text-popover-foreground elevation-2 rounded-lg border",
  scrim: "bg-foreground/24",
  chip: "rounded-pill border px-2 py-0.5",
  headerTint: "bg-azure-50 dark:bg-azure-950",
  micro: "micro-label text-muted-foreground",
  errorText: "text-small text-red-700 dark:text-red-300",
  helpText: "text-small text-muted-foreground",
} as const;
