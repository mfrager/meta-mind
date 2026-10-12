export type MapEngine = "radial" | "graph" | "outline";

export function MapToolbar({
  engine,
  onEngine,
  onExport,
}: {
  engine: MapEngine;
  onEngine: (engine: MapEngine) => void;
  onExport: (format: "svg" | "json" | "md" | "png") => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2" data-map-toolbar>
      {(["radial", "graph", "outline"] as const).map((item) => (
        <button key={item} type="button" aria-pressed={engine === item} className="text-small" onClick={() => onEngine(item)}>
          {item}
        </button>
      ))}
      {(["svg", "json", "md", "png"] as const).map((format) => (
        <button key={format} type="button" className="text-small" onClick={() => onExport(format)}>
          {format}
        </button>
      ))}
    </div>
  );
}
