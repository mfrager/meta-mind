import { fetchJson, withQuery } from "@/lib/api/client.ts";
import type {
  DataEntity,
  DataSearchHit,
  EntityPage,
  NamespaceBinding,
  SchemaSummary,
} from "@/lib/wire/inspector.ts";

export function fetchEntities(params: {
  q?: string;
  class?: string;
  limit?: number;
  offset?: number;
  includeDeleted?: boolean;
}): Promise<EntityPage> {
  return fetchJson<EntityPage>(
    withQuery("/api/console/data/entities", {
      q: params.q,
      class: params.class,
      limit: params.limit ?? 20,
      offset: params.offset,
      includeDeleted: params.includeDeleted ? "true" : undefined,
    }),
  );
}

export function isEntity(item: DataEntity | DataSearchHit): item is DataEntity {
  return "properties" in item && "types" in item;
}

export function fetchSchemas(): Promise<{ items: SchemaSummary[] }> {
  return fetchJson("/api/console/data/schemas");
}

export function fetchNamespaces(): Promise<{ items: NamespaceBinding[] }> {
  return fetchJson("/api/console/data/namespaces");
}
