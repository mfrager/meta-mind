import type { StatusValue } from "@/lib/wire/inspector.ts";

export function ago(at: number | null | undefined, now = Date.now()): string {
  if (!at) return "not yet";
  const ms = Math.max(0, now - at);
  if (ms < 1_000) return "just now";
  if (ms < 60_000) return `${Math.round(ms / 1_000)}s ago`;
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m ago`;
  if (ms < 86_400_000) return `${Math.round(ms / 3_600_000)}h ago`;
  return `${Math.round(ms / 86_400_000)}d ago`;
}

export function clockAt(at: number | null | undefined): string {
  if (!at) return "not set";
  return new Date(at).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function until(at: number | null | undefined, now = Date.now()): string {
  if (!at) return "not set";
  const ms = at - now;
  if (ms <= 0) return "due now";
  if (ms < 60_000) return `in ${Math.ceil(ms / 1_000)}s`;
  if (ms < 3_600_000) return `in ${Math.ceil(ms / 60_000)}m`;
  if (ms < 86_400_000) return `in ${Math.ceil(ms / 3_600_000)}h`;
  return `in ${Math.ceil(ms / 86_400_000)}d`;
}

export function compactTokens(value: number): string {
  if (value < 1_000) return String(value);
  if (value < 1_000_000) return `${(value / 1_000).toFixed(value < 10_000 ? 1 : 0)}k`;
  return `${(value / 1_000_000).toFixed(1)}m`;
}

export function shortId(id: string): string {
  const tail = id.includes("/") ? id.slice(id.lastIndexOf("/") + 1) : id;
  return tail.length > 12 ? `${tail.slice(0, 6)}…${tail.slice(-4)}` : tail;
}

export function asString(value: StatusValue | undefined): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

export function asBoolean(value: StatusValue | undefined): boolean | null {
  return typeof value === "boolean" ? value : null;
}

export function asNumber(value: StatusValue | undefined): number | null {
  return typeof value === "number" ? value : null;
}

export function asList(value: StatusValue | undefined): string[] {
  if (Array.isArray(value)) {
    return value.flatMap((item) => {
      if (typeof item === "string") return [item];
      if (typeof item === "number" || typeof item === "boolean") return [String(item)];
      return [];
    });
  }
  if (typeof value === "string") return [value];
  return [];
}

export function asRecord(value: StatusValue | undefined): Record<string, StatusValue> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, StatusValue>)
    : null;
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
