import { NextResponse } from "next/server";
import {
  buildRustyRedDataPayload,
  fixtureRustyRedDataPayload,
  normalizeCommonplaceRustyRedViewId,
  type CommonplaceGraphqlBriefing,
  type CommonplaceGraphqlCandidateLink,
  type CommonplaceGraphqlCollection,
  type CommonplaceGraphqlItem,
  type CommonplaceRustyRedViewId,
} from "@/lib/commonplace/rustyred-data-contract";

const DEFAULT_COMMONPLACE_GRAPHQL_URL = "https://rustyredcore-theorem-production.up.railway.app/graphql";
const DEFAULT_TIMEOUT_MS = 10_000;

interface RustyRedRouteRequest {
  view?: unknown;
}

interface CommonplaceRustyRedGraphqlResponse {
  data?: {
    items?: readonly CommonplaceGraphqlItem[];
    collections?: readonly CommonplaceGraphqlCollection[];
    briefing?: CommonplaceGraphqlBriefing | null;
    discover?: readonly CommonplaceGraphqlCandidateLink[];
  };
  errors?: readonly { message?: string }[];
}

export async function GET(request: Request) {
  const url = new URL(request.url);
  return rustyRedDataResponse(normalizeCommonplaceRustyRedViewId(url.searchParams.get("view")));
}

export async function POST(request: Request) {
  let body: RustyRedRouteRequest = {};
  try {
    body = (await request.json()) as RustyRedRouteRequest;
  } catch {
    return NextResponse.json(
      fixtureRustyRedDataPayload("files", "Expected a JSON request body."),
      { status: 400 },
    );
  }

  return rustyRedDataResponse(normalizeCommonplaceRustyRedViewId(body.view));
}

async function rustyRedDataResponse(view: CommonplaceRustyRedViewId) {
  const endpoint = normalizeGraphqlEndpoint(commonplaceGraphqlEndpointBase());
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs());

  try {
    const upstream = await fetch(endpoint, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Accept: "application/json",
        ...authorizationHeaders(endpoint),
      },
      body: JSON.stringify({
        operationName: "CommonplaceRustyRedData",
        query: COMMONPLACE_RUSTYRED_QUERY,
        variables: variablesForView(view),
      }),
      signal: controller.signal,
      cache: "no-store",
    });
    const text = await upstream.text();
    const payload = parseGraphqlPayload(text);
    const errors = payload.errors?.map((error) => error.message).filter((message): message is string => Boolean(message));

    if (!upstream.ok || errors?.length || !payload.data) {
      const message = errors?.join("; ") || `CommonPlace GraphQL returned ${upstream.status} ${upstream.statusText || "without JSON data"}.`;
      return NextResponse.json(fixtureRustyRedDataPayload(view, message), { status: 200 });
    }

    return NextResponse.json(
      buildRustyRedDataPayload({
        view,
        items: payload.data?.items ?? [],
        collections: payload.data?.collections ?? [],
        briefing: payload.data?.briefing ?? null,
        candidateLinks: payload.data?.discover ?? [],
        source: {
          mode: "live",
          endpoint: publicEndpointLabel(endpoint),
        },
      }),
    );
  } catch (error) {
    const aborted = error instanceof Error && error.name === "AbortError";
    const message = aborted ? "CommonPlace GraphQL request timed out." : errorMessage(error);
    return NextResponse.json(fixtureRustyRedDataPayload(view, message), { status: 200 });
  } finally {
    clearTimeout(timeout);
  }
}

function commonplaceGraphqlEndpointBase() {
  return (
    process.env.COMMONPLACE_GRAPHQL_URL ??
    process.env.THEOREM_COMMONPLACE_GRAPHQL_URL ??
    process.env.COMMONPLACE_API_URL ??
    process.env.NEXT_PUBLIC_COMMONPLACE_API_URL ??
    process.env.THEOREM_API_URL ??
    process.env.NEXT_PUBLIC_THEOREM_API_URL ??
    process.env.NEXT_PUBLIC_HARNESS_URL ??
    DEFAULT_COMMONPLACE_GRAPHQL_URL
  );
}

function normalizeGraphqlEndpoint(raw: string) {
  const url = new URL(raw);
  const path = url.pathname.replace(/\/+$/, "");
  if (path.endsWith("/graphql")) {
    url.pathname = path;
    return url.toString();
  }

  const base = path
    .replace(/\/v1\/theorem\/agent\/run$/i, "")
    .replace(/\/api\/theorem\/agent$/i, "")
    .replace(/\/mcp$/i, "");
  url.pathname = `${base}/graphql`.replace(/\/{2,}/g, "/");
  return url.toString();
}

function timeoutMs() {
  const raw = process.env.COMMONPLACE_GRAPHQL_TIMEOUT_MS;
  const parsed = raw ? Number(raw) : NaN;
  if (!Number.isFinite(parsed) || parsed <= 0) return DEFAULT_TIMEOUT_MS;
  return Math.min(parsed, 60_000);
}

function authorizationHeaders(endpoint: string): Record<string, string> {
  const token =
    process.env.COMMONPLACE_API_KEY ??
    process.env.THEOREM_COMMONPLACE_API_KEY ??
    process.env.THEOREM_API_TOKEN ??
    process.env.THEOREM_AGENT_API_TOKEN ??
    process.env.HARNESS_API_KEY ??
    localDefaultApiKey(endpoint);
  return token ? { "x-api-key": token, Authorization: `Bearer ${token}` } : {};
}

function localDefaultApiKey(endpoint: string) {
  const { hostname } = new URL(endpoint);
  return hostname === "localhost" || hostname === "127.0.0.1" ? "dev-key" : undefined;
}

function variablesForView(view: CommonplaceRustyRedViewId) {
  return {
    kind: null,
    recentLimit: view === "timeline" ? 30 : 14,
    connectedLimit: view === "graph" ? 30 : 12,
    openLimit: 12,
    minSimilarity: 0.5,
    maxResults: view === "graph" ? 40 : 16,
  };
}

function publicEndpointLabel(endpoint: string) {
  const url = new URL(endpoint);
  return `${url.origin}${url.pathname}`;
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function parseGraphqlPayload(text: string): CommonplaceRustyRedGraphqlResponse {
  if (!text.trim()) return {};
  try {
    return JSON.parse(text) as CommonplaceRustyRedGraphqlResponse;
  } catch {
    return { errors: [{ message: "CommonPlace GraphQL returned a non-JSON response." }] };
  }
}

const ITEM_FIELDS = `
  id
  kind
  title
  bodyText
  blobHash
  mime
  source
  residency
  tags
  collections
  classification
  path
  createdAtMs
  updatedAtMs
`;

const COMMONPLACE_RUSTYRED_QUERY = `
  query CommonplaceRustyRedData(
    $kind: String
    $recentLimit: Int
    $connectedLimit: Int
    $openLimit: Int
    $minSimilarity: Float
    $maxResults: Int
  ) {
    items(kind: $kind) {
      ${ITEM_FIELDS}
    }
    collections {
      id
      name
      kind
      createdAtMs
    }
    briefing(recentLimit: $recentLimit, connectedLimit: $connectedLimit, openLimit: $openLimit) {
      recent {
        ${ITEM_FIELDS}
      }
      newlyConnected {
        item {
          ${ITEM_FIELDS}
        }
        connections
        related {
          ${ITEM_FIELDS}
        }
      }
      openThreads {
        ${ITEM_FIELDS}
      }
    }
    discover(minSimilarity: $minSimilarity, maxResults: $maxResults) {
      a {
        ${ITEM_FIELDS}
      }
      b {
        ${ITEM_FIELDS}
      }
      similarity
      reason
    }
  }
`;
