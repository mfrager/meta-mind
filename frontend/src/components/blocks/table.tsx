import { DataTable } from "@/components/kit/data.tsx";
import { FilterForm } from "@/components/kit/forms.tsx";

export function TableBlock() {
  return (
    <div data-token="table-block" className="flex flex-col gap-3">
      <FilterForm idPrefix="table-filter" />
      <DataTable
        rows={[
          { id: "1", name: "sqlite", state: "ok" },
          { id: "2", name: "eventlog", state: "ok" },
          { id: "3", name: "graph", state: "unavailable" },
        ]}
      />
    </div>
  );
}
