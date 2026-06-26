import { HARNESS_MCP_PATH, HARNESS_SOURCE, HARNESS_URL } from "@/lib/harness";
import type { SpaceTypeInstance, SpaceTypeRepository } from "./types";

const STORE_KEY = "harness-console:space-types:v1";

export const BUILT_IN_SPACE_TYPES: SpaceTypeInstance[] = [
  space("space:home", "home", "Home", 10),
  space("space:auto-organizer", "auto-organizer", "Auto-Organizer", 20),
  space("space:browser", "browser", "Browser", 30),
  space("space:code", "code", "Code", 40),
  space("space:code:workspace", "code-workspace", "Workspace", 45, "space:code"),
  space("space:notes", "notes", "Notes", 50),
  space("space:canvas", "canvas", "Canvas", 60),
  space("space:accounts", "accounts", "Accounts", 100),
  space("space:accounts:agents", "agents", "Agents", 110, "space:accounts"),
  space("space:agents:thread", "agent-thread", "Agent", 111, "space:accounts:agents"),
  space("space:agents:memory", "memory", "Memory", 112, "space:accounts:agents"),
  space("space:agents:skills", "skills", "Skills", 113, "space:accounts:agents"),
  space("space:agents:rooms", "rooms", "Rooms", 114, "space:accounts:agents"),
  space("space:agents:runs", "runs", "Runs", 115, "space:accounts:agents"),
  space("space:accounts:mcp-hub", "mcp-hub", "MCP Hub", 120, "space:accounts"),
  space("space:mcp-hub:providers", "providers", "Providers", 121, "space:accounts:mcp-hub"),
  space("space:mcp-hub:connections", "connections", "Connections", 122, "space:accounts:mcp-hub"),
  space("space:accounts:api-keys", "api-keys", "API Keys", 130, "space:accounts"),
  space("space:accounts:usage", "usage", "Usage", 140, "space:accounts"),
  space("space:accounts:settings", "settings", "Settings", 150, "space:accounts"),
  space("space:plugin:collection", "generic-collection", "Collection", 900, undefined, false),
];

function space(
  id: string,
  typeKey: string,
  label: string,
  order: number,
  parent?: string,
  enabled = true,
): SpaceTypeInstance {
  return {
    id,
    typeKey,
    label,
    order,
    enabled,
    parent,
    config: { type_key: typeKey },
  };
}

class DevSpaceTypeRepository implements SpaceTypeRepository {
  async list(): Promise<SpaceTypeInstance[]> {
    return readDevStore();
  }

  async save(instance: SpaceTypeInstance): Promise<SpaceTypeInstance> {
    const next = upsert(await this.list(), instance);
    writeDevStore(next);
    return instance;
  }

  async rename(id: string, label: string): Promise<SpaceTypeInstance[]> {
    const next = (await this.list()).map((item) => (item.id === id ? { ...item, label } : item));
    writeDevStore(next);
    return next;
  }

  async reorder(activeId: string, overId: string): Promise<SpaceTypeInstance[]> {
    const next = reorderInstances(await this.list(), activeId, overId);
    writeDevStore(next);
    return next;
  }

  async setEnabled(id: string, enabled: boolean): Promise<SpaceTypeInstance[]> {
    const next = (await this.list()).map((item) => (item.id === id ? { ...item, enabled } : item));
    writeDevStore(next);
    return next;
  }

  async create(input: Omit<SpaceTypeInstance, "id" | "order" | "enabled"> & Partial<Pick<SpaceTypeInstance, "id" | "order" | "enabled">>): Promise<SpaceTypeInstance[]> {
    const current = await this.list();
    const order = input.order ?? Math.max(0, ...current.map((item) => item.order)) + 10;
    const id = input.id ?? `space:${input.typeKey}:${Date.now().toString(16)}`;
    const next = upsert(current, {
      ...input,
      id,
      order,
      enabled: input.enabled ?? true,
    });
    writeDevStore(next);
    return next;
  }
}

class GraphqlSpaceTypeRepository implements SpaceTypeRepository {
  constructor(private readonly fallback: SpaceTypeRepository) {}

  async list(): Promise<SpaceTypeInstance[]> {
    try {
      const data = await graphql("graphql_query", `query{ itemsByKind(kind:"space_type", limit:250){ id title extra } }`);
      const items = readGraphqlData(data, "itemsByKind").map(itemFromGraphql).filter(isSpaceInstance);
      if (items.length > 0) {
        return mergeBuiltIns(items);
      }
    } catch {
      return this.fallback.list();
    }
    return this.fallback.list();
  }

  async save(instance: SpaceTypeInstance): Promise<SpaceTypeInstance> {
    try {
      await graphql(
        "graphql_mutate",
        `mutation($input:ItemInput!){ putItem(input:$input){ id } }`,
        { input: itemInput(instance) },
      );
      await this.fallback.save(instance);
      return instance;
    } catch {
      return this.fallback.save(instance);
    }
  }

  async rename(id: string, label: string): Promise<SpaceTypeInstance[]> {
    const current = await this.list();
    const instance = current.find((item) => item.id === id);
    if (!instance) return current;
    await this.save({ ...instance, label });
    return this.list();
  }

  async reorder(activeId: string, overId: string): Promise<SpaceTypeInstance[]> {
    const next = reorderInstances(await this.list(), activeId, overId);
    await Promise.all(next.map((instance) => this.save(instance)));
    return next;
  }

  async setEnabled(id: string, enabled: boolean): Promise<SpaceTypeInstance[]> {
    const current = await this.list();
    const instance = current.find((item) => item.id === id);
    if (!instance) return current;
    await this.save({ ...instance, enabled });
    return this.list();
  }

  async create(input: Omit<SpaceTypeInstance, "id" | "order" | "enabled"> & Partial<Pick<SpaceTypeInstance, "id" | "order" | "enabled">>): Promise<SpaceTypeInstance[]> {
    const current = await this.list();
    const instance: SpaceTypeInstance = {
      id: input.id ?? `space:${input.typeKey}:${Date.now().toString(16)}`,
      typeKey: input.typeKey,
      label: input.label,
      parent: input.parent,
      config: input.config,
      order: input.order ?? Math.max(0, ...current.map((item) => item.order)) + 10,
      enabled: input.enabled ?? true,
    };
    await this.save(instance);
    return this.list();
  }
}

export function createSpaceTypeRepository(): SpaceTypeRepository {
  const dev = new DevSpaceTypeRepository();
  return HARNESS_SOURCE === "live" ? new GraphqlSpaceTypeRepository(dev) : dev;
}

function readDevStore(): SpaceTypeInstance[] {
  if (typeof window === "undefined") {
    return mergeBuiltIns([]);
  }
  const raw = window.localStorage.getItem(STORE_KEY);
  if (!raw) {
    const seeded = mergeBuiltIns([]);
    writeDevStore(seeded);
    return seeded;
  }
  try {
    return mergeBuiltIns(JSON.parse(raw) as SpaceTypeInstance[]);
  } catch {
    const seeded = mergeBuiltIns([]);
    writeDevStore(seeded);
    return seeded;
  }
}

function writeDevStore(instances: SpaceTypeInstance[]): void {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(STORE_KEY, JSON.stringify(sortInstances(instances)));
}

function mergeBuiltIns(instances: SpaceTypeInstance[]): SpaceTypeInstance[] {
  const byId = new Map(instances.map((item) => [item.id, item]));
  for (const builtIn of BUILT_IN_SPACE_TYPES) {
    if (!byId.has(builtIn.id)) {
      byId.set(builtIn.id, builtIn);
    }
  }
  return sortInstances([...byId.values()]);
}

function upsert(instances: SpaceTypeInstance[], instance: SpaceTypeInstance): SpaceTypeInstance[] {
  const replaced = instances.some((item) => item.id === instance.id);
  const next = replaced
    ? instances.map((item) => (item.id === instance.id ? instance : item))
    : [...instances, instance];
  return sortInstances(next);
}

function reorderInstances(instances: SpaceTypeInstance[], activeId: string, overId: string): SpaceTypeInstance[] {
  if (activeId === overId) return instances;
  const active = instances.find((item) => item.id === activeId);
  const over = instances.find((item) => item.id === overId);
  if (!active || !over) return instances;
  const parent = over.parent;
  const siblings = sortInstances(
    instances
      .filter((item) => item.id !== activeId)
      .filter((item) => (item.parent ?? "") === (parent ?? "")),
  );
  const targetIndex = siblings.findIndex((item) => item.id === overId);
  const moved = { ...active, parent };
  siblings.splice(Math.max(targetIndex, 0), 0, moved);
  const reordered = siblings.map((item, index) => ({ ...item, order: (index + 1) * 10 }));
  const changed = new Map(reordered.map((item) => [item.id, item]));
  return sortInstances(instances.map((item) => changed.get(item.id) ?? item));
}

function sortInstances(instances: SpaceTypeInstance[]): SpaceTypeInstance[] {
  return [...instances].sort((a, b) => a.order - b.order || a.label.localeCompare(b.label));
}

async function graphql(
  op: "graphql_query" | "graphql_mutate",
  query: string,
  variables: Record<string, unknown> = {},
): Promise<unknown> {
  const res = await fetch(`${HARNESS_URL}${HARNESS_MCP_PATH}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: Date.now(),
      method: "tools/call",
      params: { name: op, arguments: { query, variables } },
    }),
  });
  if (!res.ok) throw new Error(`space type graphql ${res.status}`);
  const payload = (await res.json()) as { result?: unknown; error?: { message?: string } };
  if (payload.error) throw new Error(payload.error.message ?? "space type graphql failed");
  return payload.result;
}

function readGraphqlData(payload: unknown, field: string): unknown[] {
  const value = payload as { structuredContent?: { data?: Record<string, unknown> }; data?: Record<string, unknown> };
  const data = value.structuredContent?.data ?? value.data;
  const items = data?.[field];
  return Array.isArray(items) ? items : [];
}

function itemFromGraphql(value: unknown): SpaceTypeInstance | null {
  if (!value || typeof value !== "object") return null;
  const item = value as { id?: unknown; title?: unknown; extra?: unknown };
  const extra = readItemExtra(item.extra);
  const typeKey = stringValue(extra.type_key) ?? stringValue(extra.typeKey);
  const id = stringValue(item.id);
  if (!id || !typeKey) return null;
  return {
    id,
    typeKey,
    label: stringValue(item.title) ?? stringValue(extra.label) ?? typeKey,
    order: numberValue(extra.order) ?? 0,
    enabled: booleanValue(extra.enabled) ?? true,
    parent: stringValue(extra.parent),
    config: objectValue(extra.config) ?? {},
  };
}

function itemInput(instance: SpaceTypeInstance): Record<string, unknown> {
  return {
    id: instance.id,
    kind: "space_type",
    title: instance.label,
    source: "commonplace:space-type-registry",
    extra: {
      type_key: instance.typeKey,
      label: instance.label,
      order: instance.order,
      enabled: instance.enabled,
      parent: instance.parent,
      config: instance.config,
    },
  };
}

function readItemExtra(value: unknown): Record<string, unknown> {
  const object = objectValue(value) ?? {};
  const nested = objectValue(object.extra);
  return nested ?? object;
}

function isSpaceInstance(value: SpaceTypeInstance | null): value is SpaceTypeInstance {
  return value !== null;
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function numberValue(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function booleanValue(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function objectValue(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}
