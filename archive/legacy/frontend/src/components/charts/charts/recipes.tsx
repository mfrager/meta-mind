import type { ReactNode } from "react";
import { Area, AreaChart, Bar, BarChart, Line, LineChart, PolarAngleAxis, PolarGrid, Radar, RadarChart, ResponsiveContainer } from "recharts";

import type { ChartSeries } from "@/components/charts/export.ts";

export const CHART_RECIPES = [
  "self_model_divergence",
  "budget_burn",
  "calibration_curve",
  "firewall_signals",
  "risk_octagon",
  "memory_growth",
  "llm_cache",
  "loop_throughput",
  "policy_fitness",
  "debt_trend",
] as const;

export type ChartRecipeId = (typeof CHART_RECIPES)[number];

function rows(series: ChartSeries[]) {
  const xs = series[0]?.points.map((point) => point.x) ?? [];
  return xs.map((x, index) => {
    const row: Record<string, string | number> = { x };
    for (const item of series) row[item.name] = item.points[index]?.y ?? 0;
    return row;
  });
}

export function ChartRecipe({ recipe, series }: { recipe: string; series: ChartSeries[] }) {
  const data = rows(series);
  const names = series.map((item) => item.name);
  if (recipe === "risk_octagon") {
    const radar = names.map((name) => ({ axis: name, value: series.find((item) => item.name === name)?.points.at(-1)?.y ?? 0 }));
    return (
      <div className="h-48" data-chart-recipe={recipe} role="img" aria-label={recipe}>
        <ResponsiveContainer width="100%" height="100%">
          <RadarChart data={radar}>
            <PolarGrid />
            <PolarAngleAxis dataKey="axis" />
            <Radar dataKey="value" stroke="var(--color-cyan-500)" fill="var(--color-cyan-500)" fillOpacity={0.2} />
          </RadarChart>
        </ResponsiveContainer>
      </div>
    );
  }
  if (recipe === "budget_burn" || recipe === "memory_growth") {
    return (
      <Frame recipe={recipe}>
        <AreaChart data={data}>
          {names.map((name) => (
            <Area key={name} dataKey={name} stroke="var(--color-cyan-500)" fill="var(--color-cyan-100)" />
          ))}
        </AreaChart>
      </Frame>
    );
  }
  if (recipe === "firewall_signals" || recipe === "debt_trend" || recipe === "loop_throughput") {
    return (
      <Frame recipe={recipe}>
        <BarChart data={data}>
          {names.map((name) => (
            <Bar key={name} dataKey={name} fill="var(--color-indigo-500)" />
          ))}
        </BarChart>
      </Frame>
    );
  }
  return (
    <Frame recipe={recipe}>
      <LineChart data={data}>
        {names.map((name) => (
          <Line key={name} dataKey={name} stroke="var(--color-cyan-500)" dot={false} strokeWidth={1.5} />
        ))}
      </LineChart>
    </Frame>
  );
}

function Frame({ recipe, children }: { recipe: string; children: ReactNode }) {
  return (
    <div className="h-48" data-chart-recipe={recipe} role="img" aria-label={recipe}>
      <ResponsiveContainer width="100%" height="100%">{children}</ResponsiveContainer>
    </div>
  );
}
