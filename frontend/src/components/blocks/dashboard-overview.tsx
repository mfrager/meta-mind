import { ChartCard, DataTable, Sparkline } from "@/components/kit/data.tsx";
import { MeterCard, StatCard } from "@/components/kit/cards.tsx";

const ROWS = [
  { id: "1", name: "sqlite", state: "ok" },
  { id: "2", name: "graph", state: "unavailable" },
];

export function DashboardOverview() {
  return (
    <div data-token="dashboard-overview" className="flex flex-col gap-3">
      <div className="grid gap-3 md:grid-cols-3">
        <StatCard />
        <MeterCard />
        <div className="border-border rounded-xs border p-3">
          <p className="micro-label">Trend</p>
          <Sparkline values={[1, 3, 2, 4, 4]} label="Event trend" />
        </div>
      </div>
      <ChartCard />
      <DataTable rows={ROWS} />
    </div>
  );
}
