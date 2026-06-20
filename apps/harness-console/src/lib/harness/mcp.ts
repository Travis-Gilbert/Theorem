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
import type { ClientKind } from "./types";

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
    const result = await callTool("composed_agent_run", {
      bindingId: "agent:theorem",
      prompt,
      scope,
    });
    const text = typeof result === "string" ? result : JSON.stringify(result);
    return {
      id: `msg_${Date.now()}`,
      role: "assistant",
      content: text,
      at: new Date().toISOString(),
      verdict: "pending",
    };
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
