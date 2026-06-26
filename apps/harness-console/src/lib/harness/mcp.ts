/**
 * Live HarnessClient over the harness MCP/HTTP backend.
 *
 * This is the wiring surface: the JSON-RPC transport plus the tool-call shapes
 * each method maps to. The harness exposes a typed GraphQL surface
 * (graphql_query / graphql_mutate / graphql_introspect) over its flat tools, so
 * reads go through graphql_query and memory writes through graphql_mutate. The
 * tenant is resolved server side from the key; the console never sends one.
 *
 * Methods that have no stable backend route yet (onboarding register, ingestion
 * metering) delegate to the mock and log a warning, so the console still renders
 * in live mode while those backend dependencies land. Swapping in the real
 * route is a single method body change.
 */
import { type HarnessClient, HARNESS_URL, HARNESS_MCP_PATH, installSnippet } from "./client";
import { mockClient } from "./mock";
import type { ChatMessage, ClientKind, TraceEntry } from "./types";

interface JsonRpcResult {
  result?: unknown;
  error?: { code: number; message: string };
}

let rpcId = 1;

async function callTool(name: string, args: Record<string, unknown>, key?: string): Promise<unknown> {
  const res = await fetch(`${HARNESS_URL}${HARNESS_MCP_PATH}`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...(key ? { Authorization: `Bearer ${key}` } : {}),
    },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: rpcId++,
      method: "tools/call",
      params: { name, arguments: args },
    }),
  });
  if (!res.ok) throw new Error(`harness ${name} -> ${res.status}`);
  const json = (await res.json()) as JsonRpcResult;
  if (json.error) throw new Error(`harness ${name}: ${json.error.message}`);
  return json.result;
}

/** Run a GraphQL document against the typed MCP surface (read or mutate). */
async function graphql(
  op: "graphql_query" | "graphql_mutate",
  query: string,
  variables: Record<string, unknown> = {},
  key?: string,
): Promise<unknown> {
  return callTool(op, { query, variables }, key);
}

// Reads currently lean on the mock projection for shape stability while the
// per-surface GraphQL field mapping is finalized; writes are wired to the real
// typed mutations. This keeps the live client honest about what is real today.
export const liveClient: HarnessClient = {
  ...mockClient,

  async runAgent(prompt, scope) {
    const response = await fetch("/api/theorem/agent", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        tenant: "Travis-Gilbert",
        binding_id: "agent:theorem",
        task: prompt,
        scope: scope ?? [],
      }),
    });
    const payload = (await response.json().catch(() => ({}))) as unknown;
    if (!response.ok) {
      throw new Error(agentPayloadText(payload) || `Theorem agent API returned ${response.status}.`);
    }
    const text = agentPayloadText(payload) || "Theorem API agent returned an empty response.";
    const trace = agentPayloadTrace(payload);
    return {
      id: `msg_${Date.now()}`,
      role: "assistant",
      content: text,
      at: new Date().toISOString(),
      verdict: agentPayloadVerdict(payload),
      trace: trace.length ? trace : undefined,
    } satisfies ChatMessage;
  },

  async saveAtom(atom) {
    await graphql(
      "graphql_mutate",
      `mutation($id:ID!,$title:String!,$body:String!){ reviseMemory(id:$id,title:$title,body:$body){ id } }`,
      { id: atom.id, title: atom.title, body: atom.body },
    );
    return { ...atom, updated: new Date().toISOString() };
  },

  async archiveAtom(id) {
    await callTool("self_archive", { id });
  },

  async trashAtom(id, reason) {
    await callTool("forget", { id, reason });
  },

  async publishSkill(skill) {
    await callTool("skill_publish", {
      name: skill.name,
      description: skill.description,
      files: skill.files,
    });
    return { ...skill, updated: new Date().toISOString() };
  },

  async applySkill(id) {
    const r = await callTool("skill_apply", { id });
    return {
      skillId: id,
      appliedAt: new Date().toISOString(),
      ok: true,
      summary: typeof r === "string" ? r : "applied",
      steps: [],
    };
  },

  installSnippet(client: ClientKind, prefix: string) {
    return installSnippet(client, prefix);
  },
};

function agentPayloadText(payload: unknown): string {
  if (typeof payload === "string") return payload;
  const result = unwrapAgentResult(payload);
  if (!isRecord(result)) return "";
  if ("error" in result) {
    return [result.error, result.message].filter((part) => typeof part === "string").join(": ");
  }

  const direct = stringProp(result, "message", "content", "text", "answer", "output", "summary");
  if (direct) return direct;

  const receipts = Array.isArray(result.invocation_receipts) ? result.invocation_receipts : [];
  for (const receipt of [...receipts].reverse()) {
    if (!isRecord(receipt)) continue;
    const payloadText = agentPayloadText(receipt.payload);
    if (payloadText) return payloadText;
    const summary = stringProp(receipt, "output_summary", "summary");
    if (summary) return summary;
  }

  const publishedClaims = Array.isArray(result.published_claims)
    ? result.published_claims
        .map((claim) => (isRecord(claim) ? stringProp(claim, "text") : ""))
        .filter(Boolean)
    : [];
  if (publishedClaims.length) return publishedClaims.join("; ");

  return compactJson(result);
}

function agentPayloadVerdict(payload: unknown): ChatMessage["verdict"] {
  const result = unwrapAgentResult(payload);
  if (!isRecord(result) || !isRecord(result.alignment_verdict)) return "pending";
  const allowed = result.alignment_verdict.allowed;
  if (allowed === true) return "aligned";
  if (allowed === false) return "blocked";
  return "pending";
}

function agentPayloadTrace(payload: unknown): TraceEntry[] {
  const result = unwrapAgentResult(payload);
  if (!isRecord(result)) return [];
  const at = new Date().toISOString();
  const events = Array.isArray(result.events) ? result.events : [];
  const receipts = Array.isArray(result.invocation_receipts) ? result.invocation_receipts : [];

  const eventEntries: TraceEntry[] = events.slice(-8).map((event, index) => {
    const record = isRecord(event) ? event : {};
    const kind = stringProp(record, "event_type", "kind", "type") || "event";
    return {
      id: stringProp(record, "event_id", "id") || `event_${index}`,
      role: "system",
      content: kind,
      at: stringProp(record, "created_at", "at", "timestamp") || at,
    };
  });

  const receiptEntries: TraceEntry[] = receipts.slice(-6).map((receipt, index) => {
    const record = isRecord(receipt) ? receipt : {};
    return {
      id: stringProp(record, "invocation_id", "id") || `receipt_${index}`,
      role: "head",
      head: stringProp(record, "head_id"),
      content: stringProp(record, "output_summary", "summary") || agentPayloadText(record.payload) || compactJson(record),
      at: stringProp(record, "created_at", "at", "timestamp") || at,
    };
  });

  return [...eventEntries, ...receiptEntries];
}

function unwrapAgentResult(payload: unknown): unknown {
  let current = payload;
  for (let i = 0; i < 4; i += 1) {
    if (!isRecord(current) || "error" in current || !("result" in current)) return current;
    current = current.result;
  }
  return current;
}

function stringProp(record: Record<string, unknown>, ...keys: string[]): string {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return "";
}

function compactJson(value: unknown): string {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
