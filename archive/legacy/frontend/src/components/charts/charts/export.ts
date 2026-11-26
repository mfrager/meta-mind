export type ChartPoint = { x: string; y: number };
export type ChartSeries = { name: string; points: ChartPoint[] };

/** CSV for a chart snapshot. Fixture series stay labeled by their series name. */
export function chartCsv(series: ChartSeries[]): string {
  const lines = ["name,x,y"];
  for (const item of series) {
    for (const point of item.points) {
      lines.push(`${item.name},${point.x},${point.y}`);
    }
  }
  return `${lines.join("\n")}\n`;
}

/** Draw the first series to a PNG blob. Returns null when canvas is unavailable. */
export function chartPng(series: ChartSeries[], caption: string): Promise<Blob | null> {
  const canvas = document.createElement("canvas");
  canvas.width = 640;
  canvas.height = 280;
  const ctx = canvas.getContext("2d");
  if (!ctx) return Promise.resolve(null);
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  const points = series[0]?.points ?? [];
  const max = Math.max(1, ...points.map((point) => point.y));
  ctx.strokeStyle = "#0891b2";
  ctx.lineWidth = 2;
  ctx.beginPath();
  points.forEach((point, index) => {
    const x = 40 + (index * (canvas.width - 80)) / Math.max(1, points.length - 1);
    const y = canvas.height - 40 - (point.y / max) * (canvas.height - 80);
    if (index === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  });
  ctx.stroke();
  ctx.fillStyle = "#334155";
  ctx.font = "12px monospace";
  ctx.fillText(caption, 16, 20);
  return new Promise((resolve) => canvas.toBlob((blob) => resolve(blob)));
}

export function download(filename: string, blob: Blob) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  URL.revokeObjectURL(url);
}
