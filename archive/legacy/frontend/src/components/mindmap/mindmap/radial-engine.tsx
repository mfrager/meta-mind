import type { MapPoint } from "@/lib/trees/layout.ts";

const FILL: Record<string, string> = {
  cyan: "#0891b2",
  azure: "#2563eb",
  violet: "#7c3aed",
  teal: "#0f766e",
  indigo: "#4338ca",
  rose: "#e11d48",
  green: "#15803d",
  amber: "#b45309",
};

export function RadialEngine({ points, total }: { points: MapPoint[]; total: number }) {
  return (
    <svg viewBox="-800 -800 1600 1600" className="h-[28rem] w-full" data-radial role="img" aria-label="Radial mind map" data-map-rendered={points.length} data-map-total={total}>
      {points.map((point) =>
        point.lod === "dot" ? (
          <circle key={point.id} data-map-node={point.id} data-lod="dot" cx={point.x} cy={point.y} r={4} fill={FILL[point.hue] ?? "#0891b2"} />
        ) : (
          <text key={point.id} data-map-node={point.id} data-lod={point.lod} x={point.x} y={point.y} textAnchor="middle" className="fill-foreground text-value">
            {point.label}
          </text>
        ),
      )}
    </svg>
  );
}
