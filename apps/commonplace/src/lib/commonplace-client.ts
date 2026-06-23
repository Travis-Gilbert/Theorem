"use client";

export interface CommonPlaceItem {
  id: string;
  kind: string;
  title: string;
  source: string;
  updatedAtMs: number;
  extra?: unknown;
}

export interface EngineConfig {
  graphqlUrl: string;
  changefeedUrl: string;
  tenant: string;
}

const LOCAL_GRAPHQL_URL = "http://127.0.0.1:17888/graphql";
const LOCAL_CHANGEFEED_URL = "http://127.0.0.1:17888/v1/items/stream";
const HOSTED_GRAPHQL_URL =
  process.env.NEXT_PUBLIC_COMMONPLACE_GRAPHQL_URL ??
  "https://rustyredcore-theorem-production.up.railway.app/graphql";
const HOSTED_CHANGEFEED_URL =
  process.env.NEXT_PUBLIC_COMMONPLACE_CHANGEFEED_URL ??
  "https://rustyredcore-theorem-production.up.railway.app/v1/items/stream";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function engineConfig(): EngineConfig {
  const tenant = process.env.NEXT_PUBLIC_COMMONPLACE_TENANT ?? "default";
  if (isTauri()) {
    return {
      graphqlUrl: LOCAL_GRAPHQL_URL,
      changefeedUrl: LOCAL_CHANGEFEED_URL,
      tenant
    };
  }
  return {
    graphqlUrl: HOSTED_GRAPHQL_URL,
    changefeedUrl: HOSTED_CHANGEFEED_URL,
    tenant
  };
}

interface GraphqlResponse<T> {
  data?: T;
  errors?: Array<{ message: string }>;
}

async function graphql<T>(
  query: string,
  variables: Record<string, unknown> = {},
  config = engineConfig()
): Promise<T> {
  const response = await fetch(`${config.graphqlUrl}?tenant=${encodeURIComponent(config.tenant)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ query, variables })
  });
  const payload = (await response.json()) as GraphqlResponse<T>;
  if (!response.ok || payload.errors?.length) {
    throw new Error(payload.errors?.map((error) => error.message).join("; ") || response.statusText);
  }
  if (!payload.data) throw new Error("GraphQL response did not include data.");
  return payload.data;
}

export async function fetchItems(): Promise<CommonPlaceItem[]> {
  const data = await graphql<{ items: CommonPlaceItem[] }>(`
    query CommonPlaceItems {
      items(limit: 60) {
        id
        kind
        title
        source
        updatedAtMs
        extra
      }
    }
  `);
  return data.items;
}

export async function createNote(title: string): Promise<CommonPlaceItem> {
  const data = await graphql<{ putItem: CommonPlaceItem }>(
    `
      mutation CreateCommonPlaceNote($input: ItemInput!) {
        putItem(input: $input) {
          id
          kind
          title
          source
          updatedAtMs
          extra
        }
      }
    `,
    {
      input: {
        kind: "note",
        title,
        source: "commonplace-desktop",
        extra: { capture: "desktop" }
      }
    }
  );
  return data.putItem;
}

export function subscribeToItemChanges(onChange: () => void): () => void {
  if (typeof window === "undefined" || typeof EventSource === "undefined") return () => {};
  const config = engineConfig();
  const source = new EventSource(`${config.changefeedUrl}?tenant=${encodeURIComponent(config.tenant)}`);
  source.addEventListener("item.upserted", onChange);
  source.addEventListener("item.deleted", onChange);
  source.onerror = () => {
    source.close();
  };
  return () => source.close();
}
