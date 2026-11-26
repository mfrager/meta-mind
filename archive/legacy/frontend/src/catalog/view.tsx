import { useEffect, useState } from "react";

import { catalogEntries, catalogGroups } from "@/catalog/registry.tsx";
import { COMMANDS, commandTier } from "@/lib/commands.ts";

export function ComponentCatalog() {
  const [token, setToken] = useState("Move over a control.");
  const [copied, setCopied] = useState("");

  useEffect(() => {
    const reset = () => {
      const active = document.activeElement;
      if (active instanceof HTMLElement && active.closest("[data-catalog]")) active.blur();
      document.querySelector(".pane-scroll")?.scrollTo(0, 0);
    };
    reset();
    const frame = requestAnimationFrame(reset);
    return () => cancelAnimationFrame(frame);
  }, []);

  return (
    <section
      className="flex flex-col gap-4"
      data-catalog
      data-catalog-count={catalogEntries.length}
      onMouseOver={(event) => {
        const node = event.target instanceof Element ? event.target.closest("[data-token]") : null;
        const value = node?.getAttribute("data-token");
        if (value) setToken(value);
      }}
    >
      <div>
        <h2 className="text-h2">Components</h2>
        <p className="text-small text-muted-foreground mt-1 max-w-xl">
          Every control, in every state it has. A sentence marks a state that does not apply.
        </p>
        <p className="text-small mt-2" data-token-inspector>
          Token <span className="font-mono text-value">{token}</span>
        </p>
      </div>
      <nav aria-label="Catalog index" className="flex flex-col gap-3">
        {catalogGroups.map((group) => (
          <div key={group}>
            <p className="micro-label">{group}</p>
            <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1">
              {catalogEntries
                .filter((entry) => entry.group === group)
                .map((entry) => (
                  <a key={entry.id} href={`#catalog-${entry.id}`} className="text-small text-primary underline-offset-2 hover:underline">
                    {entry.name}
                  </a>
                ))}
            </div>
          </div>
        ))}
      </nav>
      {catalogEntries.map((entry) => (
        <section key={entry.id} id={`catalog-${entry.id}`} className="flex flex-col gap-3" data-catalog-entry={entry.id}>
          <div>
            <h3 className="text-h3">{entry.name}</h3>
            <p className="micro-label text-muted-foreground">{entry.from}</p>
          </div>
          <div className={entry.group === "Blocks" ? "grid gap-3" : "grid gap-3 md:grid-cols-2 xl:grid-cols-4"}>
            {entry.states.map((state) => (
              <div key={state.id} data-specimen={`${entry.id}-${state.id}`} className="border-border min-w-0 rounded-xs border p-3">
                <div className="mb-2 flex items-center justify-between gap-2">
                  <p className="micro-label">{state.label}</p>
                  <button
                    type="button"
                    className="text-small text-primary"
                    onClick={() => {
                      const mark = `${entry.id}-${state.id}`;
                      void navigator.clipboard?.writeText(state.jsx).then(() => setCopied(mark));
                    }}
                  >
                    {copied === `${entry.id}-${state.id}` ? "Copied" : "Copy JSX"}
                  </button>
                </div>
                {state.node}
                <pre className="text-muted-foreground mt-2 overflow-auto font-mono text-micro">{state.jsx}</pre>
              </div>
            ))}
          </div>
        </section>
      ))}
      <section data-parity data-parity-count={COMMANDS.length} className="flex flex-col gap-2">
        <h3 className="text-h3">Command parity</h3>
        <p className="text-small text-muted-foreground">Each CLI subcommand is bespoke or a registered passthrough.</p>
        <ul className="grid gap-1 sm:grid-cols-2">
          {COMMANDS.map((command) => (
            <li key={command.id} data-command-tier={commandTier(command.id)} className="font-mono text-xs">
              {command.argv} · {commandTier(command.id)}
            </li>
          ))}
        </ul>
      </section>
    </section>
  );
}
