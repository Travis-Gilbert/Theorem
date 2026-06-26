import { NextResponse } from "next/server";

const DEFAULT_THEOREM_URL = "https://rustyredcore-theorem-production.up.railway.app";
const DEFAULT_TENANT = "Travis-Gilbert";
const DEFAULT_BINDING_ID = "agent:theorem";
const DEFAULT_TIMEOUT_MS = 12_000;

interface AgentRequestBody {
  task?: unknown;
  prompt?: unknown;
  tenant?: unknown;
  tenant_slug?: unknown;
  binding_id?: unknown;
  bindingId?: unknown;
  claims?: unknown;
  scope?: unknown;
}

export async function POST(request: Request) {
  let body: AgentRequestBody;
  try {
    body = (await request.json()) as AgentRequestBody;
  } catch {
    return NextResponse.json({ error: "invalid_json", message: "Expected a JSON request body." }, { status: 400 });
  }

  const task = stringValue(body.task) ?? stringValue(body.prompt);
  if (!task?.trim()) {
    return NextResponse.json({ error: "invalid_agent_run", message: "Agent run requires task." }, { status: 400 });
  }

  const endpoint = normalizeTheoremAgentEndpoint(theoremAgentEndpointBase());
  const controller = new AbortController();
  const timeout = windowlessTimeout(() => controller.abort(), theoremAgentTimeoutMs());

  try {
    const headers: HeadersInit = {
      "Content-Type": "application/json",
      ...authorizationHeader(),
    };
    const upstream = await fetch(endpoint, {
      method: "POST",
      headers,
      body: JSON.stringify({
        tenant: stringValue(body.tenant) ?? stringValue(body.tenant_slug) ?? DEFAULT_TENANT,
        binding_id: stringValue(body.binding_id) ?? stringValue(body.bindingId) ?? DEFAULT_BINDING_ID,
        task,
        claims: Array.isArray(body.claims) ? body.claims : [],
        scope: Array.isArray(body.scope) ? body.scope : [],
      }),
      signal: controller.signal,
    });

    const text = await upstream.text();
    const payload = parseJsonPayload(text);
    return NextResponse.json(payload, { status: upstream.status });
  } catch (error) {
    const aborted = error instanceof Error && error.name === "AbortError";
    return NextResponse.json(
      {
        error: aborted ? "theorem_agent_timeout" : "theorem_agent_proxy_failed",
        message: aborted ? "Theorem agent run exceeded the console response window." : errorMessage(error),
      },
      { status: aborted ? 504 : 502 },
    );
  } finally {
    clearTimeout(timeout);
  }
}

export function normalizeTheoremAgentEndpoint(raw: string) {
  const url = new URL(raw);
  const path = url.pathname.replace(/\/+$/, "");

  if (path.endsWith("/v1/theorem/agent/run")) {
    url.pathname = path;
    return url.toString();
  }

  const base = path.replace(/\/(?:graphql|mcp|api\/theorem\/agent)$/i, "");
  url.pathname = `${base}/v1/theorem/agent/run`.replace(/\/{2,}/g, "/");
  return url.toString();
}

function theoremAgentEndpointBase() {
  return (
    process.env.THEOREM_AGENT_ENDPOINT ??
    process.env.THEOREM_API_URL ??
    process.env.NEXT_PUBLIC_THEOREM_API_URL ??
    process.env.NEXT_PUBLIC_HARNESS_URL ??
    DEFAULT_THEOREM_URL
  );
}

function theoremAgentTimeoutMs() {
  const raw = process.env.THEOREM_AGENT_HTTP_TIMEOUT_MS ?? process.env.THEOREM_AGENT_HTTP_TIMEOUT_SECS;
  const parsed = raw ? Number(raw) : NaN;
  if (!Number.isFinite(parsed) || parsed <= 0) return DEFAULT_TIMEOUT_MS;
  return raw === process.env.THEOREM_AGENT_HTTP_TIMEOUT_SECS ? Math.min(parsed * 1000, 60_000) : Math.min(parsed, 60_000);
}

function authorizationHeader(): Record<string, string> {
  const token = process.env.THEOREM_AGENT_API_TOKEN ?? process.env.THEOREM_API_TOKEN ?? process.env.HARNESS_API_KEY;
  return token ? { Authorization: `Bearer ${token}` } : {};
}

function stringValue(value: unknown) {
  return typeof value === "string" ? value : undefined;
}

function parseJsonPayload(text: string) {
  if (!text) return {};
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return { raw: text };
  }
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function windowlessTimeout(callback: () => void, ms: number) {
  return setTimeout(callback, ms);
}
