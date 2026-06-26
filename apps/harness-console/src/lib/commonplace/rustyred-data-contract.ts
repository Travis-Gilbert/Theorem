import type {
  JsonValue,
  ObjectQuery,
  ObjectRef,
  ObjectShape,
  Ranker,
  TypeRef,
} from "@/lib/block-view/types";

export const COMMONPLACE_RUSTYRED_DATA_CONTRACT_VERSION = "commonplace-rustyred-data/v1";

export type CommonplaceRustyRedViewId = "files" | "graph" | "table" | "timeline" | "clips" | "map";
export type CommonplaceRustyRedSourceMode = "live" | "fixture" | "error";
export type CommonplaceNocoBaseBridgeStatus = "installed" | "configured" | "needs-service";

export interface CommonplaceGraphqlItem {
  id: string;
  kind: string;
  title: string;
  bodyText?: string | null;
  blobHash?: string | null;
  mime?: string | null;
  source?: string | null;
  residency: string;
  tags: readonly string[];
  collections: readonly string[];
  classification?: string | null;
  path?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
}

export interface CommonplaceGraphqlCollection {
  id: string;
  name: string;
  kind: string;
  createdAtMs: number;
}

export interface CommonplaceGraphqlConnectedItem {
  item: CommonplaceGraphqlItem;
  connections: number;
  related: readonly CommonplaceGraphqlItem[];
}

export interface CommonplaceGraphqlBriefing {
  recent: readonly CommonplaceGraphqlItem[];
  newlyConnected: readonly CommonplaceGraphqlConnectedItem[];
  openThreads: readonly CommonplaceGraphqlItem[];
}

export interface CommonplaceGraphqlCandidateLink {
  a: CommonplaceGraphqlItem;
  b: CommonplaceGraphqlItem;
  similarity: number;
  reason: string;
}

export interface CommonplaceSerializableObjectSet {
  readonly objects: readonly ObjectRef[];
  readonly shape: ObjectShape;
  readonly next_cursor?: string;
}

export interface CommonplaceRustyRedGraphNode {
  id: string;
  label: string;
  type: TypeRef;
  x: number;
  y: number;
  color: [number, number, number];
  size: number;
  meta?: string;
}

export interface CommonplaceRustyRedGraphLink {
  source: string;
  target: string;
  edge: "IN_COLLECTION" | "SIMILAR_TO" | "BRIEFING_RELATED";
  weight?: number;
  reason?: string;
}

export interface CommonplaceRustyRedTableRow {
  id: string;
  title: string;
  type: TypeRef;
  status: string;
  source: string;
  updatedAt: string;
  collections: string;
  tags: string;
}

export interface CommonplaceRustyRedGeoPoint {
  id: string;
  label: string;
  coordinates: readonly [longitude: number, latitude: number];
  radius: number;
  type: TypeRef;
}

export interface CommonplaceNocoBaseFieldContract {
  name: string;
  type: "string" | "text" | "integer" | "datetime" | "json" | "many-to-many";
  sourceField: string;
}

export interface CommonplaceNocoBaseCollectionContract {
  name: string;
  title: string;
  sourceType: TypeRef;
  fields: readonly CommonplaceNocoBaseFieldContract[];
}

export interface CommonplaceNocoBaseBridge {
  packageName: "@nocobase/client";
  mode: "isolated-client-app";
  status: CommonplaceNocoBaseBridgeStatus;
  dataSource: "commonplace-graphql" | "rustyred-pg-wire";
  collections: readonly CommonplaceNocoBaseCollectionContract[];
  records: readonly CommonplaceRustyRedTableRow[];
  notes: readonly string[];
}

export interface CommonplaceRustyRedDataSource {
  mode: CommonplaceRustyRedSourceMode;
  endpoint?: string;
  message?: string;
}

export interface CommonplaceRustyRedDataPayload {
  version: typeof COMMONPLACE_RUSTYRED_DATA_CONTRACT_VERSION;
  view: CommonplaceRustyRedViewId;
  generatedAt: string;
  source: CommonplaceRustyRedDataSource;
  request: {
    objectQuery: ObjectQuery;
    graphql: {
      operationName: "CommonplaceRustyRedData";
      fields: readonly string[];
    };
  };
  objectSet: CommonplaceSerializableObjectSet;
  items: readonly CommonplaceGraphqlItem[];
  collections: readonly CommonplaceGraphqlCollection[];
  candidateLinks: readonly CommonplaceGraphqlCandidateLink[];
  graph: {
    nodes: readonly CommonplaceRustyRedGraphNode[];
    links: readonly CommonplaceRustyRedGraphLink[];
  };
  table: {
    rows: readonly CommonplaceRustyRedTableRow[];
  };
  geo: {
    status: "available" | "deferred";
    points: readonly CommonplaceRustyRedGeoPoint[];
    renderer: "@deck.gl/react";
    note: string;
  };
  nocobase: CommonplaceNocoBaseBridge;
  errors?: readonly string[];
}

export interface BuildRustyRedDataPayloadInput {
  view: CommonplaceRustyRedViewId;
  items?: readonly CommonplaceGraphqlItem[];
  collections?: readonly CommonplaceGraphqlCollection[];
  briefing?: CommonplaceGraphqlBriefing | null;
  candidateLinks?: readonly CommonplaceGraphqlCandidateLink[];
  source: CommonplaceRustyRedDataSource;
  generatedAt?: string;
  errors?: readonly string[];
}

const GRAPHQL_FIELDS = [
  "items",
  "collections",
  "briefing.recent",
  "briefing.newlyConnected",
  "briefing.openThreads",
  "discover",
] as const;

const NOOP_COLLECTIONS: readonly CommonplaceGraphqlCollection[] = [];
const NOOP_LINKS: readonly CommonplaceGraphqlCandidateLink[] = [];

export function normalizeCommonplaceRustyRedViewId(value: unknown): CommonplaceRustyRedViewId {
  if (value === "files" || value === "graph" || value === "table" || value === "timeline" || value === "clips" || value === "map") {
    return value;
  }
  return "files";
}

export function buildRustyRedDataPayload(input: BuildRustyRedDataPayloadInput): CommonplaceRustyRedDataPayload {
  const allItems = dedupeItems([
    ...(input.items ?? []),
    ...(input.briefing?.recent ?? []),
    ...(input.briefing?.openThreads ?? []),
    ...(input.briefing?.newlyConnected.flatMap((entry) => [entry.item, ...entry.related]) ?? []),
    ...(input.candidateLinks ?? NOOP_LINKS).flatMap((link) => [link.a, link.b]),
  ]);
  const viewItems = selectItemsForView(input.view, allItems);
  const collections = input.collections ?? NOOP_COLLECTIONS;
  const candidateLinks = input.candidateLinks ?? NOOP_LINKS;
  const briefingLinks = briefingToGraphLinks(input.briefing);
  const graph = itemsToGraph(viewItems, collections, candidateLinks, briefingLinks);
  const tableRows = itemsToTableRows(viewItems);
  const geoPoints = itemsToGeoPoints(viewItems);

  return {
    version: COMMONPLACE_RUSTYRED_DATA_CONTRACT_VERSION,
    view: input.view,
    generatedAt: input.generatedAt ?? new Date().toISOString(),
    source: input.source,
    request: {
      objectQuery: objectQueryForView(input.view),
      graphql: {
        operationName: "CommonplaceRustyRedData",
        fields: GRAPHQL_FIELDS,
      },
    },
    objectSet: itemsToObjectSet(viewItems, collections, candidateLinks, briefingLinks, input.view),
    items: viewItems,
    collections,
    candidateLinks,
    graph,
    table: {
      rows: tableRows,
    },
    geo: {
      status: "deferred",
      points: geoPoints,
      renderer: "@deck.gl/react",
      note: "Deck.gl is installed for object-coordinate layers. The Map surface and MapLibre base map are intentionally deferred.",
    },
    nocobase: itemsToNocoBaseBridge(tableRows),
    errors: input.errors,
  };
}

export function fixtureRustyRedDataPayload(
  view: CommonplaceRustyRedViewId,
  message?: string,
): CommonplaceRustyRedDataPayload {
  return buildRustyRedDataPayload({
    view,
    items: [
      fixtureItem({
        id: "fixture:file:spec-commonplace-ia",
        kind: "file",
        title: "SPEC-COMMONPLACE-INFORMATION-ARCHITECTURE.md",
        path: "/Specs/SPEC-COMMONPLACE-INFORMATION-ARCHITECTURE.md",
        tags: ["commonplace", "ia"],
      }),
      fixtureItem({
        id: "fixture:note:code-contract",
        kind: "note",
        title: "Code harness block contract",
        bodyText: "The block and view contract normalizes RustyRed objects into trusted renderers.",
        tags: ["code", "contract"],
      }),
      fixtureItem({
        id: "fixture:clip:assistant-ui",
        kind: "clip",
        title: "assistant-ui primitives",
        source: "https://assistant-ui.com/docs/primitives",
        tags: ["clip", "agent"],
      }),
      fixtureItem({
        id: "fixture:task:graph-renderer",
        kind: "task",
        title: "Wire RustyRed graph renderer",
        classification: "active",
        tags: ["graph", "cosmos"],
      }),
      fixtureItem({
        id: "fixture:artifact:scene-package",
        kind: "artifact",
        title: "ScenePackage preview",
        path: "/Artifacts/scene-package.json",
        tags: ["sceneos"],
      }),
    ],
    collections: [
      { id: "collection:code", name: "Code", kind: "collection", createdAtMs: Date.UTC(2026, 5, 25) },
      { id: "collection:data", name: "Data", kind: "collection", createdAtMs: Date.UTC(2026, 5, 25) },
    ],
    candidateLinks: [
      {
        a: fixtureItem({ id: "fixture:note:code-contract", kind: "note", title: "Code harness block contract" }),
        b: fixtureItem({ id: "fixture:task:graph-renderer", kind: "task", title: "Wire RustyRed graph renderer" }),
        similarity: 0.84,
        reason: "Shared block/view renderer contract",
      },
    ],
    source: {
      mode: message ? "error" : "fixture",
      message: message ?? "Fixture data is active until CommonPlace GraphQL is reachable.",
    },
    errors: message ? [message] : undefined,
  });
}

function objectQueryForView(view: CommonplaceRustyRedViewId): ObjectQuery {
  const types = typesForView(view);
  const rank: Ranker[] = [{ kind: "field", field: "updatedAtMs", direction: "desc" }];
  if (view === "graph" || view === "clips") {
    rank.push({ kind: "fulltext", query: view, fields: ["title", "bodyPreview", "tags"] });
  }

  return {
    types,
    traverse: view === "graph" ? [{ edge: "SIMILAR_TO", dir: "out" }, { edge: "IN_COLLECTION", dir: "out", target: "collection" }] : undefined,
    rank,
    project: {
      fields: ["title", "kind", "source", "path", "tags", "classification", "updatedAtMs"],
      relations: [
        { edge: "IN_COLLECTION", dir: "out", target: "collection" },
        { edge: "SIMILAR_TO", dir: "out" },
      ],
      include_body_preview: true,
      include_metadata: true,
    },
    page: { limit: 50 },
    live: true,
  };
}

function typesForView(view: CommonplaceRustyRedViewId): readonly TypeRef[] {
  if (view === "files") return ["file", "doc", "image"];
  if (view === "clips") return ["clip", "web_capture", "link", "media"];
  if (view === "timeline") return ["event", "task", "thread", "artifact", "note", "file"];
  if (view === "map") return ["place", "event", "asset"];
  if (view === "table") return ["record", "task", "project", "schema", "note", "file", "clip", "artifact"];
  return ["node", "edge", "cluster", "record", "task", "project", "note", "file", "clip", "artifact"];
}

function itemsToObjectSet(
  items: readonly CommonplaceGraphqlItem[],
  collections: readonly CommonplaceGraphqlCollection[],
  candidateLinks: readonly CommonplaceGraphqlCandidateLink[],
  briefingLinks: readonly CommonplaceRustyRedGraphLink[],
  view: CommonplaceRustyRedViewId,
): CommonplaceSerializableObjectSet {
  const collectionObjects = collections.map(collectionToObjectRef);
  const itemObjects = items.map((item) => itemToObjectRef(item, candidateLinks, briefingLinks));
  const objects = [...itemObjects, ...collectionObjects];

  return {
    objects,
    shape: {
      types: typesForView(view),
      fields: ["title", "kind", "source", "path", "tags", "classification", "createdAtMs", "updatedAtMs"],
      relations: [
        { edge: "IN_COLLECTION", dir: "out", target: "collection" },
        { edge: "SIMILAR_TO", dir: "out" },
        { edge: "BRIEFING_RELATED", dir: "out" },
      ],
      axes: {
        temporal: true,
        spatial: view === "map",
        embeddable: true,
      },
      cardinality: objects.length === 0 ? "empty" : objects.length === 1 ? "one" : "many",
    },
  };
}

function itemToObjectRef(
  item: CommonplaceGraphqlItem,
  candidateLinks: readonly CommonplaceGraphqlCandidateLink[],
  briefingLinks: readonly CommonplaceRustyRedGraphLink[],
): ObjectRef {
  const similar = candidateLinks
    .filter((link) => link.a.id === item.id || link.b.id === item.id)
    .map((link) => (link.a.id === item.id ? link.b.id : link.a.id));
  const briefingRelated = briefingLinks
    .filter((link) => link.source === item.id || link.target === item.id)
    .map((link) => (link.source === item.id ? link.target : link.source));

  return {
    id: item.id,
    type: normalizeType(item.kind),
    properties: {
      title: item.title,
      kind: item.kind,
      bodyPreview: preview(item.bodyText),
      blobHash: item.blobHash ?? null,
      mime: item.mime ?? null,
      source: item.source ?? null,
      residency: item.residency,
      tags: stringArray(item.tags),
      collections: stringArray(item.collections),
      classification: item.classification ?? null,
      path: item.path ?? null,
      createdAtMs: item.createdAtMs,
      updatedAtMs: item.updatedAtMs,
    },
    relations: {
      IN_COLLECTION: item.collections,
      SIMILAR_TO: similar,
      BRIEFING_RELATED: briefingRelated,
    },
    axes: {
      valid: { from_ms: item.createdAtMs, to_ms: item.updatedAtMs },
      embeddable: true,
    },
  };
}

function collectionToObjectRef(collection: CommonplaceGraphqlCollection): ObjectRef {
  return {
    id: collection.id,
    type: "collection",
    properties: {
      title: collection.name,
      kind: collection.kind,
      createdAtMs: collection.createdAtMs,
    },
    axes: {
      valid: { from_ms: collection.createdAtMs },
    },
  };
}

function itemsToGraph(
  items: readonly CommonplaceGraphqlItem[],
  collections: readonly CommonplaceGraphqlCollection[],
  candidateLinks: readonly CommonplaceGraphqlCandidateLink[],
  briefingLinks: readonly CommonplaceRustyRedGraphLink[],
) {
  const itemNodes = items.map((item): CommonplaceRustyRedGraphNode => ({
    id: item.id,
    label: item.title,
    type: normalizeType(item.kind),
    x: stableUnit(item.id, "x"),
    y: stableUnit(item.id, "y"),
    color: colorForKind(item.kind),
    size: 4 + Math.min(5, item.collections.length + item.tags.length),
    meta: [item.kind, item.source, item.path].filter(Boolean).join(" / "),
  }));
  const collectionNodes = collections.map((collection): CommonplaceRustyRedGraphNode => ({
    id: collection.id,
    label: collection.name,
    type: "collection",
    x: stableUnit(collection.id, "x"),
    y: stableUnit(collection.id, "y"),
    color: [0.72, 0.45, 0.22],
    size: 7,
    meta: collection.kind,
  }));
  const collectionLinks = items.flatMap((item) =>
    item.collections.map((collectionId): CommonplaceRustyRedGraphLink => ({
      source: item.id,
      target: collectionId,
      edge: "IN_COLLECTION",
      weight: 1,
    })),
  );
  const similarLinks = candidateLinks.map((link): CommonplaceRustyRedGraphLink => ({
    source: link.a.id,
    target: link.b.id,
    edge: "SIMILAR_TO",
    weight: link.similarity,
    reason: link.reason,
  }));

  return {
    nodes: [...itemNodes, ...collectionNodes],
    links: dedupeGraphLinks([...collectionLinks, ...similarLinks, ...briefingLinks]),
  };
}

function briefingToGraphLinks(briefing?: CommonplaceGraphqlBriefing | null): readonly CommonplaceRustyRedGraphLink[] {
  if (!briefing) return [];
  return briefing.newlyConnected.flatMap((entry) =>
    entry.related.map((related): CommonplaceRustyRedGraphLink => ({
      source: entry.item.id,
      target: related.id,
      edge: "BRIEFING_RELATED",
      weight: entry.connections,
    })),
  );
}

function dedupeGraphLinks(links: readonly CommonplaceRustyRedGraphLink[]) {
  const seen = new Set<string>();
  return links.filter((link) => {
    const key = `${link.edge}:${link.source}:${link.target}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function itemsToTableRows(items: readonly CommonplaceGraphqlItem[]): readonly CommonplaceRustyRedTableRow[] {
  return [...items]
    .sort((a, b) => b.updatedAtMs - a.updatedAtMs)
    .map((item) => ({
      id: item.id,
      title: item.title,
      type: normalizeType(item.kind),
      status: item.classification ?? item.residency ?? "unclassified",
      source: item.source ?? item.path ?? item.mime ?? "rustyred",
      updatedAt: formatDate(item.updatedAtMs),
      collections: item.collections.length ? item.collections.join(", ") : "none",
      tags: item.tags.length ? item.tags.join(", ") : "none",
    }));
}

function itemsToGeoPoints(items: readonly CommonplaceGraphqlItem[]): readonly CommonplaceRustyRedGeoPoint[] {
  return items
    .map((item) => {
      const coordinates = coordinatesFromItem(item);
      if (!coordinates) return undefined;
      return {
        id: item.id,
        label: item.title,
        coordinates,
        radius: 64 + Math.round(stableUnit(item.id, "radius") * 160),
        type: normalizeType(item.kind),
      } satisfies CommonplaceRustyRedGeoPoint;
    })
    .filter((point): point is CommonplaceRustyRedGeoPoint => Boolean(point));
}

function itemsToNocoBaseBridge(rows: readonly CommonplaceRustyRedTableRow[]): CommonplaceNocoBaseBridge {
  return {
    packageName: "@nocobase/client",
    mode: "isolated-client-app",
    status: "installed",
    dataSource: "commonplace-graphql",
    collections: [
      {
        name: "rustyred_objects",
        title: "RustyRed Objects",
        sourceType: "record",
        fields: [
          { name: "title", type: "string", sourceField: "title" },
          { name: "type", type: "string", sourceField: "type" },
          { name: "status", type: "string", sourceField: "classification" },
          { name: "source", type: "string", sourceField: "source" },
          { name: "updated_at", type: "datetime", sourceField: "updatedAtMs" },
          { name: "collections", type: "many-to-many", sourceField: "collections" },
          { name: "tags", type: "json", sourceField: "tags" },
        ],
      },
    ],
    records: rows,
    notes: [
      "NocoBase is treated as an isolated app/plugin boundary, not copied componentry.",
      "The current bridge is read-ready over CommonPlace GraphQL; a writable NocoBase service should mount as a separate data-source runtime.",
    ],
  };
}

function selectItemsForView(
  view: CommonplaceRustyRedViewId,
  items: readonly CommonplaceGraphqlItem[],
): readonly CommonplaceGraphqlItem[] {
  if (view === "graph" || view === "table") return newestFirst(items).slice(0, 50);
  if (view === "timeline") return newestFirst(items).slice(0, 30);
  if (view === "files") return newestFirst(items.filter(isFileLike)).slice(0, 30);
  if (view === "clips") return newestFirst(items.filter(isClipLike)).slice(0, 30);
  return newestFirst(items.filter((item) => item.kind === "place" || item.kind === "event")).slice(0, 30);
}

function isFileLike(item: CommonplaceGraphqlItem) {
  return Boolean(item.path || item.blobHash || item.mime || ["file", "doc", "image"].includes(item.kind));
}

function isClipLike(item: CommonplaceGraphqlItem) {
  return ["clip", "web_capture", "link", "media"].includes(item.kind) || Boolean(item.source?.startsWith("http"));
}

function newestFirst(items: readonly CommonplaceGraphqlItem[]) {
  return [...items].sort((a, b) => b.updatedAtMs - a.updatedAtMs);
}

function dedupeItems(items: readonly CommonplaceGraphqlItem[]): readonly CommonplaceGraphqlItem[] {
  const byId = new Map<string, CommonplaceGraphqlItem>();
  for (const item of items) {
    if (item?.id) byId.set(item.id, normalizeItem(item));
  }
  return [...byId.values()];
}

function normalizeItem(item: CommonplaceGraphqlItem): CommonplaceGraphqlItem {
  return {
    id: item.id,
    kind: item.kind || "record",
    title: item.title || item.id,
    bodyText: item.bodyText ?? null,
    blobHash: item.blobHash ?? null,
    mime: item.mime ?? null,
    source: item.source ?? null,
    residency: item.residency || "local",
    tags: stringArray(item.tags),
    collections: stringArray(item.collections),
    classification: item.classification ?? null,
    path: item.path ?? null,
    createdAtMs: Number.isFinite(item.createdAtMs) ? item.createdAtMs : Date.now(),
    updatedAtMs: Number.isFinite(item.updatedAtMs) ? item.updatedAtMs : Date.now(),
  };
}

function fixtureItem(input: Partial<CommonplaceGraphqlItem> & { id: string; kind: string; title: string }): CommonplaceGraphqlItem {
  const baseTime = Date.UTC(2026, 5, 25, 12, 0, 0);
  return {
    id: input.id,
    kind: input.kind,
    title: input.title,
    bodyText: input.bodyText ?? null,
    blobHash: input.blobHash ?? null,
    mime: input.mime ?? null,
    source: input.source ?? "fixture",
    residency: input.residency ?? "local",
    tags: input.tags ?? [],
    collections: input.collections ?? ["collection:data"],
    classification: input.classification ?? "ready",
    path: input.path ?? null,
    createdAtMs: input.createdAtMs ?? baseTime,
    updatedAtMs: input.updatedAtMs ?? baseTime + Math.round(stableUnit(input.id, "time") * 86_400_000),
  };
}

function normalizeType(kind: string): TypeRef {
  return kind.trim().toLowerCase().replace(/[^a-z0-9_:-]+/g, "_") || "record";
}

function preview(value?: string | null): JsonValue {
  if (!value) return null;
  return value.length > 180 ? `${value.slice(0, 177)}...` : value;
}

function stringArray(values: readonly string[] | undefined): readonly string[] {
  return (values ?? []).filter((value): value is string => typeof value === "string" && Boolean(value.trim()));
}

function colorForKind(kind: string): [number, number, number] {
  const normalized = normalizeType(kind);
  if (normalized === "file" || normalized === "doc" || normalized === "image") return [0.26, 0.43, 0.58];
  if (normalized === "task" || normalized === "project") return [0.74, 0.34, 0.24];
  if (normalized === "clip" || normalized === "link" || normalized === "web_capture") return [0.19, 0.43, 0.47];
  if (normalized === "artifact") return [0.65, 0.45, 0.22];
  return [0.43, 0.34, 0.28];
}

function stableUnit(id: string, salt: string) {
  const hash = fnv1a(`${salt}:${id}`);
  return (hash % 10_000) / 10_000;
}

function fnv1a(value: string) {
  let hash = 0x811c9dc5;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

function coordinatesFromItem(item: CommonplaceGraphqlItem): readonly [number, number] | undefined {
  const haystack = [item.title, item.bodyText, item.source, item.path].filter(Boolean).join(" ");
  const match = haystack.match(/(-?\d{1,3}\.\d+)\s*,\s*(-?\d{1,3}\.\d+)/);
  if (!match) return undefined;
  const longitude = Number(match[1]);
  const latitude = Number(match[2]);
  if (!Number.isFinite(longitude) || !Number.isFinite(latitude)) return undefined;
  if (longitude < -180 || longitude > 180 || latitude < -90 || latitude > 90) return undefined;
  return [longitude, latitude];
}

function formatDate(ms: number) {
  if (!Number.isFinite(ms)) return "unknown";
  return new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" }).format(new Date(ms));
}
