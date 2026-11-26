import { GaugeArc } from "@/components/kit/gauge-arc.tsx";

export const IMAGE_DEGRADED =
  "this route records images as attachments; the model will see captions only";

function recordOf(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function reasonOf(value: unknown) {
  const reason = recordOf(value).reason;
  return typeof reason === "string" ? reason : "";
}

export function OpBody({ name, output }: { name: string; output: unknown }) {
  const record = recordOf(output);
  const reason = reasonOf(output);
  if (name === "decide") {
    return (
      <div data-op-view="decide" className="space-y-1">
        <p className="text-xs font-medium">Decision</p>
        <p>{reason || "No decision was recorded."}</p>
      </div>
    );
  }
  if (name === "firewall eval") {
    const signals = Array.isArray(record.signals) ? record.signals : [];
    return (
      <div data-op-view="firewall" className="space-y-1">
        <p className="text-xs font-medium">Firewall</p>
        <p>{reason || "No firewall evaluation was recorded."}</p>
        <ul className="flex flex-wrap gap-1">
          {signals.map((signal) => {
            const row = recordOf(signal);
            const label = typeof row.name === "string" ? row.name : "signal";
            return (
              <li key={label} className="rounded-md border border-border px-1.5 py-0.5 text-xs">
                {label}
              </li>
            );
          })}
        </ul>
      </div>
    );
  }
  if (name === "budget show") {
    const rows = Array.isArray(record.rows) ? record.rows : [];
    return (
      <div data-op-view="budget" className="space-y-2">
        <p className="text-xs font-medium">Budgets</p>
        {reason ? <p>{reason}</p> : null}
        <ul className="flex flex-wrap gap-3">
          {rows.map((row) => {
            const item = recordOf(row);
            const kind = typeof item.kind === "string" ? item.kind : "budget";
            const limit = typeof item.limit === "number" ? item.limit : 0;
            const spent = typeof item.spent === "number" ? item.spent : 0;
            const percent = limit > 0 ? Math.round((spent / limit) * 100) : 0;
            return (
              <li key={kind}>
                <GaugeArc value={percent} label={kind} />
                <p className="text-xs">{kind}</p>
              </li>
            );
          })}
        </ul>
      </div>
    );
  }
  if (name === "verify run") {
    const steps = Array.isArray(record.steps) ? record.steps : [];
    return (
      <div data-op-view="verify" className="space-y-1">
        <p className="text-xs font-medium">Proof</p>
        <p>{reason || "No proof was recorded."}</p>
        <ol className="list-decimal pl-4 text-xs">
          {steps.map((step) => (
            <li key={String(step)}>{String(step)}</li>
          ))}
        </ol>
      </div>
    );
  }
  if (name === "sandbox run") {
    const stages = Array.isArray(record.stages) ? record.stages : [];
    return (
      <div data-op-view="sandbox" className="space-y-1">
        <p className="text-xs font-medium">Sandbox</p>
        <p>{reason || "No sandbox run was recorded."}</p>
        <ol className="text-xs">
          {stages.map((stage) => {
            const row = recordOf(stage);
            const label = typeof row.label === "string" ? row.label : "stage";
            return <li key={label}>{label}</li>;
          })}
        </ol>
      </div>
    );
  }
  return (
    <pre data-op-view="generic" className="overflow-x-auto text-xs">
      {JSON.stringify(output, null, 2)}
    </pre>
  );
}
