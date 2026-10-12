import { BRANCH_HUES } from "@/lib/trees/layout.ts";

export function MapLegend() {
  return (
    <ul data-map-legend className="text-small flex flex-wrap gap-2">
      {BRANCH_HUES.map((hue) => (
        <li key={hue}>{hue}</li>
      ))}
      <li>solid parent</li>
      <li>dashed relation</li>
    </ul>
  );
}
