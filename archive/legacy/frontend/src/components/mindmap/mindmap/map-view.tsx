import { useEffect, useState } from "react";

import { GraphEngine } from "@/components/mindmap/graph-engine.tsx";
import { MapLegend } from "@/components/mindmap/legend.tsx";
import { OutlineMode } from "@/components/mindmap/outline-mode.tsx";
import { RadialEngine } from "@/components/mindmap/radial-engine.tsx";
import { MapToolbar, type MapEngine } from "@/components/mindmap/toolbar.tsx";
import { download } from "@/components/charts/export.ts";
import { fetchMap } from "@/lib/api/trees.ts";
import { mapSvg, outlineMarkdown } from "@/lib/trees/export.ts";
import { pointsForZoom, type MapPoint } from "@/lib/trees/layout.ts";
import { treeMapRef, type TreeNode } from "@/lib/trees/model.ts";

export function MapView({ source }: { source: string }) {
  const [nodes, setNodes] = useState<TreeNode[]>([]);
  const [reason, setReason] = useState<string | null>(null);
  const [title, setTitle] = useState(source);
  const [engine, setEngine] = useState<MapEngine>("radial");
  const [zoom, setZoom] = useState(1);
  const [points, setPoints] = useState<MapPoint[]>([]);
  const [chatRef, setChatRef] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    void fetchMap(source)
      .then((map) => {
        if (cancelled) return;
        setNodes(map.nodes);
        setReason(map.reason ?? null);
        setTitle(map.title);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : "The map did not load.");
      });
    return () => {
      cancelled = true;
    };
  }, [source]);

  useEffect(() => {
    if (nodes.length < 2000) {
      setPoints(pointsForZoom(nodes, zoom));
      return;
    }
    const worker = new Worker(new URL("./layout.worker.ts", import.meta.url), { type: "module" });
    worker.onmessage = (event: MessageEvent<MapPoint[]>) => {
      setPoints(event.data);
      worker.terminate();
    };
    worker.postMessage({ nodes, zoom });
    return () => worker.terminate();
  }, [nodes, zoom]);

  function onExport(format: "svg" | "json" | "md" | "png") {
    if (format === "md") {
      download(`${source}.md`, new Blob([outlineMarkdown(nodes)], { type: "text/markdown" }));
      return;
    }
    if (format === "json") {
      download(`${source}.json`, new Blob([JSON.stringify({ id: source, nodes }, null, 2)], { type: "application/json" }));
      return;
    }
    if (format === "svg") {
      download(`${source}.svg`, new Blob([mapSvg(points)], { type: "image/svg+xml" }));
      return;
    }
    const canvas = document.createElement("canvas");
    canvas.width = 640;
    canvas.height = 360;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = "#0f172a";
    ctx.font = "12px monospace";
    points.slice(0, 40).forEach((point, index) => ctx.fillText(point.label, 16, 24 + index * 16));
    void canvas.toBlob((blob) => {
      if (blob) download(`${source}.png`, blob);
    });
  }

  const focus = nodes[0];
  return (
    <div className="flex h-full min-h-0 flex-col gap-3 p-4" data-map-source={source}>
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-h2">{title}</h2>
        <button type="button" className="text-small" onClick={() => setZoom(0.4)}>-
        </button>
        <button type="button" className="text-small" onClick={() => setZoom(1)}>fit</button>
        <button
          type="button"
          className="text-small underline"
          onClick={() => {
            if (focus) setChatRef(treeMapRef(source, focus.id));
          }}
        >
          Send to chat
        </button>
      </div>
      <MapToolbar engine={engine} onEngine={setEngine} onExport={onExport} />
      {reason ? <p data-map-reason className="text-body">{reason}</p> : null}
      {error ? <p className="text-body text-red-700 dark:text-red-300">{error}</p> : null}
      {chatRef ? <p className="font-mono text-value" data-chat-ref>{chatRef}</p> : null}
      {engine === "radial" ? <RadialEngine points={points} total={nodes.length} /> : null}
      {engine === "graph" ? <GraphEngine nodes={nodes} /> : null}
      {engine === "outline" ? <OutlineMode nodes={nodes} /> : (
        <div className="sr-only">
          <OutlineMode nodes={nodes} />
        </div>
      )}
      <MapLegend />
    </div>
  );
}
