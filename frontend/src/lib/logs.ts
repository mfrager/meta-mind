export function severityAt(index: number): "error" | "warn" | "info" {
  if (index % 50 === 0) return "error";
  if (index % 10 === 0) return "warn";
  return "info";
}

export function lineFor(index: number) {
  return `${String(index).padStart(7, "0")} ${severityAt(index)} system fixture`;
}

/** Indexed count for the fixture log. Severity and the fixed suffix are O(1). */
export function countMatching(query: string, count: number): number {
  const q = query.trim().toLowerCase();
  if (!q || count <= 0) return q ? 0 : count;
  if (q === "error") return Math.floor((count - 1) / 50) + 1;
  if (q === "warn") return Math.floor((count - 1) / 10) - Math.floor((count - 1) / 50);
  if (q === "info") return count - countMatching("error", count) - countMatching("warn", count);
  if ("system fixture".includes(q)) return count;
  return 0;
}
