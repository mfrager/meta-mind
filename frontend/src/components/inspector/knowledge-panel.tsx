import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { SearchIcon } from "lucide-react";
import { useState } from "react";

import { fetchDataHealth } from "@/lib/api/inspector.ts";
import { fetchEntities, fetchNamespaces, fetchSchemas, isEntity } from "@/lib/api/data.ts";
import type { DataEntity, DataSearchHit, DataValue } from "@/lib/wire/inspector.ts";
import { ago, errorMessage, shortId } from "@/lib/inspector/format.ts";
import { cn } from "@/lib/utils";

import { Chip, UnavailableRow } from "./section.tsx";

type KnowledgeTab = "entities" | "schemas" | "namespaces";

function valueText(value: DataValue): string {
  if (value.iri) return value.iri;
  if (value.value === null || value.value === undefined) return "not set";
  return typeof value.value === "string" ? value.value : JSON.stringify(value.value);
}

function EntityCard({ entity }: { entity: DataEntity }) {
  return (
    <article className="border-border bg-surface/50 rounded-sm border p-2" data-entity={entity.iri}>
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <p className="truncate text-[12px] font-medium">{entity.label ?? shortId(entity.iri)}</p>
          <p className="truncate font-mono text-[10px] text-muted-foreground" title={entity.iri}>
            {entity.iri}
          </p>
        </div>
        <Chip tone={entity.status === "active" ? "ok" : "warn"}>{entity.status}</Chip>
      </div>
      <p className="mt-1 text-[10px] text-muted-foreground">
        {entity.types.join(" · ")} · rev {entity.revision} · by {entity.recorded_by}
      </p>
      <dl className="mt-1.5 flex flex-col gap-0.5">
        {Object.entries(entity.properties).map(([predicate, values]) => (
          <div className="flex min-w-0 gap-2 text-[11px]" key={predicate}>
            <dt className="w-28 shrink-0 truncate text-muted-foreground" title={predicate}>
              {shortId(predicate)}
            </dt>
            <dd className="min-w-0 flex-1 break-words">{values.map(valueText).join(" · ")}</dd>
          </div>
        ))}
      </dl>
    </article>
  );
}

function HitCard({ hit, onSelect }: { hit: DataSearchHit; onSelect: (hit: DataSearchHit) => void }) {
  return (
    <button
      className="border-border hover:bg-muted/40 w-full rounded-sm border p-2 text-left"
      onClick={() => onSelect(hit)}
      type="button"
    >
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium">
          {hit.label ?? hit.iri}
        </span>
        <Chip>{(hit.score_micros / 1_000_000).toFixed(2)}</Chip>
      </div>
      <p className="truncate font-mono text-[9px] text-muted-foreground">{hit.iri}</p>
      {hit.snippet ? <p className="mt-1 line-clamp-2 text-[11px] text-muted-foreground">{hit.snippet}</p> : null}
    </button>
  );
}

export function KnowledgePanel({ onOpenTrace }: { onOpenTrace?: (traceId: string) => void }) {
  const [tab, setTab] = useState<KnowledgeTab>("entities");
  const [draft, setDraft] = useState("");
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<DataEntity | DataSearchHit | null>(null);

  const health = useQuery({ queryKey: ["inspector", "data-health"], queryFn: fetchDataHealth });
  const schemas = useQuery({ queryKey: ["inspector", "schemas"], queryFn: fetchSchemas });
  const namespaces = useQuery({ queryKey: ["inspector", "namespaces"], queryFn: fetchNamespaces, retry: false });
  const entities = useInfiniteQuery({
    queryKey: ["inspector", "entities", search],
    initialPageParam: 0,
    queryFn: ({ pageParam }) =>
      fetchEntities({ q: search || undefined, offset: pageParam, limit: 20 }),
    getNextPageParam: (last) =>
      last.hasMore ? (last.nextOffset ?? (search ? undefined : undefined)) : undefined,
  });
  const rows = entities.data?.pages.flatMap((page) => page.items) ?? [];

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-panel="knowledge">
      <div className="border-border/70 flex items-center gap-2 border-b px-3 py-2">
        <span className="flex-1 text-[11px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
          Knowledge
        </span>
        {health.data ? (
          <Chip tone={health.data.status.index_ready ? "ok" : "warn"}>
            {health.data.status.index_ready ? "index ready" : "index pending"}
          </Chip>
        ) : null}
      </div>
      <div className="border-border/70 flex gap-0.5 border-b px-2 py-1">
        {(["entities", "schemas", "namespaces"] as const).map((id) => (
          <button
            aria-pressed={tab === id}
            className={cn(
              "h-6 rounded-sm px-2 text-[11px] font-medium",
              tab === id ? "bg-surface text-foreground shadow-elev-1" : "text-muted-foreground hover:text-foreground",
            )}
            key={id}
            onClick={() => setTab(id)}
            type="button"
          >
            {id}
          </button>
        ))}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {health.isError ? (
          <UnavailableRow section="knowledge health" reason={errorMessage(health.error)} />
        ) : null}
        {health.data ? (
          <div className="mb-2 grid grid-cols-2 gap-1 text-[11px]">
            <span>graph · {health.data.health.quads.toLocaleString()} quads</span>
            <span>{health.data.status.entities.toLocaleString()} entities</span>
            <span>{health.data.status.search_documents.toLocaleString()} docs</span>
            <span>{health.data.status.index_pending} outbox</span>
            <span>{health.data.schemas} schemas</span>
            <span>{health.data.namespaces} namespaces</span>
            <span className="col-span-2 text-muted-foreground">
              last mutation {health.data.status.last_mutation ? ago(health.data.status.last_mutation) : "not set"}
            </span>
          </div>
        ) : null}

        {tab === "entities" ? (
          <div className="flex flex-col gap-2">
            <form
              className="flex items-center gap-1"
              onSubmit={(event) => {
                event.preventDefault();
                setSearch(draft.trim());
                setSelected(null);
              }}
            >
              <label className="relative min-w-0 flex-1">
                <SearchIcon className="pointer-events-none absolute top-1.5 left-2 size-3 text-muted-foreground" />
                <input
                  aria-label="Search entities"
                  className="border-input bg-background h-6 w-full rounded-sm border pr-2 pl-6 text-[11px]"
                  onChange={(event) => setDraft(event.target.value)}
                  placeholder="Search entities…"
                  value={draft}
                />
              </label>
              <button className="text-[11px] text-primary" type="submit">
                Search
              </button>
            </form>
            {entities.isError ? (
              <UnavailableRow section="entities" reason={errorMessage(entities.error)} />
            ) : null}
            {entities.isPending ? <p className="text-[12px] text-muted-foreground">Loading entities…</p> : null}
            {!entities.isPending && rows.length === 0 ? (
              <p className="text-[12px] text-muted-foreground">
                {search ? `No entities match “${search}”.` : "No entities are recorded."}
              </p>
            ) : null}
            <div className="flex flex-col gap-1.5">
              {rows.map((item) =>
                isEntity(item) ? (
                  <button key={item.iri} onClick={() => setSelected(item)} type="button">
                    <EntityCard entity={item} />
                  </button>
                ) : (
                  <HitCard hit={item} key={item.iri} onSelect={setSelected} />
                ),
              )}
            </div>
            {entities.hasNextPage ? (
              <button
                className="text-[11px] text-primary disabled:opacity-50"
                disabled={entities.isFetchingNextPage}
                onClick={() => void entities.fetchNextPage()}
                type="button"
              >
                {entities.isFetchingNextPage ? "Loading…" : "Load more entities"}
              </button>
            ) : null}
            {selected ? (
              <div className="border-border mt-1 rounded-sm border p-2">
                <div className="mb-1 flex items-center justify-between">
                  <span className="text-[11px] font-semibold">Selected entity</span>
                  <button className="text-[11px] text-muted-foreground" onClick={() => setSelected(null)} type="button">
                    Close
                  </button>
                </div>
                {isEntity(selected) ? (
                  <EntityCard entity={selected} />
                ) : (
                  <div>
                    <p className="text-[12px] font-medium">{selected.label ?? selected.iri}</p>
                    <p className="font-mono text-[10px] text-muted-foreground">{selected.iri}</p>
                    <p className="mt-1 text-[11px]">
                      {selected.type} · rev {selected.revision} · {selected.graph}
                    </p>
                  </div>
                )}
              </div>
            ) : null}
            {onOpenTrace ? (
              <button className="self-start text-[11px] text-primary" onClick={() => onOpenTrace("main")} type="button">
                Open main trace
              </button>
            ) : null}
          </div>
        ) : null}

        {tab === "schemas" ? (
          <div className="flex flex-col gap-1.5">
            {schemas.isError ? <UnavailableRow section="schemas" reason={errorMessage(schemas.error)} /> : null}
            {schemas.data?.items.map((schema) => (
              <div className="border-border rounded-sm border p-2" key={schema.id}>
                <div className="flex items-center gap-1.5">
                  <span className="min-w-0 flex-1 truncate font-mono text-[11px]">{schema.prefix}</span>
                  <Chip>{schema.version}</Chip>
                  <Chip tone={schema.active ? "ok" : "neutral"}>{schema.active ? "active" : "inactive"}</Chip>
                </div>
                <p className="truncate font-mono text-[10px] text-muted-foreground" title={schema.namespace}>
                  {schema.namespace}
                </p>
                <p className="mt-0.5 text-[10px] text-muted-foreground">
                  {schema.class_count} classes · {schema.property_count} properties ·{" "}
                  {schema.loaded_at ? ago(schema.loaded_at) : "load time not set"}
                </p>
              </div>
            ))}
            {schemas.data?.items.length === 0 ? (
              <p className="text-[12px] text-muted-foreground">No schemas are registered.</p>
            ) : null}
          </div>
        ) : null}

        {tab === "namespaces" ? (
          <div className="flex flex-col gap-1">
            {namespaces.isError ? (
              <UnavailableRow section="namespaces" reason={errorMessage(namespaces.error)} />
            ) : null}
            {namespaces.data?.items.map((binding) => (
              <div className="flex min-w-0 items-baseline gap-1.5 text-[11px]" key={binding.prefix}>
                <span className="w-20 shrink-0 font-mono">{binding.prefix}</span>
                <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground" title={binding.namespace}>
                  {binding.namespace}
                </span>
                {binding.builtin ? <Chip>builtin</Chip> : null}
              </div>
            ))}
            {namespaces.data?.items.length === 0 ? (
              <p className="text-[12px] text-muted-foreground">No namespace bindings are recorded.</p>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}
