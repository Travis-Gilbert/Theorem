import { requestUrl, RequestUrlParam } from "obsidian";
import type { HarnessSyncSettings } from "./settings";
import type {
  HarnessDoc,
  ListDocsResponse,
  UpsertNoteArgs,
  UpsertNoteReceipt,
} from "./types";

export class HarnessError extends Error {
  status?: number;
  constructor(message: string, status?: number) {
    super(message);
    this.name = "HarnessError";
    this.status = status;
  }
}

/**
 * Thin HTTP client over the harness. Reads go through the REST list endpoint;
 * writes go through the `/mcp` JSON-RPC `upsert_note` tool. All requests use
 * Obsidian's `requestUrl` so there is no CORS problem on desktop or mobile.
 */
export class HarnessClient {
  constructor(private settings: HarnessSyncSettings) {}

  private get base(): string {
    const base = this.settings.baseUrl.trim().replace(/\/+$/, "");
    if (!base) {
      throw new HarnessError("Harness base URL is not configured.");
    }
    return base;
  }

  private get tenant(): string {
    return (this.settings.tenant || "default").trim() || "default";
  }

  private authHeaders(json: boolean): Record<string, string> {
    const headers: Record<string, string> = {};
    if (this.settings.token.trim()) {
      headers["Authorization"] = `Bearer ${this.settings.token.trim()}`;
    }
    if (json) {
      headers["Content-Type"] = "application/json";
    }
    return headers;
  }

  /** GET the tenant's memory docs, optionally only those updated at/after `since`. */
  async listDocs(since: string): Promise<ListDocsResponse> {
    const params = new URLSearchParams();
    if (since) {
      params.set("since", since);
    }
    if (this.settings.includeInactive) {
      params.set("include_inactive", "true");
    }
    const query = params.toString();
    const url =
      `${this.base}/v1/tenants/${encodeURIComponent(this.tenant)}/memory/docs` +
      (query ? `?${query}` : "");

    const response = await this.send({
      url,
      method: "GET",
      headers: this.authHeaders(false),
    });

    const body = this.parseJson(response.text);
    if (!body || body.ok !== true || !Array.isArray(body.docs)) {
      throw new HarnessError("Unexpected response from the memory list endpoint.");
    }
    return body as ListDocsResponse;
  }

  /** Upsert one note via the `upsert_note` MCP tool and return its receipt. */
  async upsertNote(args: UpsertNoteArgs): Promise<UpsertNoteReceipt> {
    const payload = {
      jsonrpc: "2.0",
      id: `obsidian-${Date.now()}`,
      method: "tools/call",
      params: {
        name: "upsert_note",
        arguments: { ...args, tenant: this.tenant },
      },
    };

    const response = await this.send({
      url: `${this.base}/mcp`,
      method: "POST",
      headers: this.authHeaders(true),
      body: JSON.stringify(payload),
    });

    const body = this.parseJson(response.text);
    if (body?.error) {
      throw new HarnessError(
        `Harness MCP error: ${body.error.message ?? JSON.stringify(body.error)}`
      );
    }
    const result = body?.result;
    const structured = result?.structuredContent ?? this.firstJsonContent(result);
    if (structured?.error) {
      throw new HarnessError(`upsert_note rejected: ${structured.message ?? structured.error}`);
    }
    const receipt = structured?.receipt as UpsertNoteReceipt | undefined;
    if (!receipt || !receipt.document) {
      throw new HarnessError("upsert_note returned no receipt.");
    }
    return receipt;
  }

  /**
   * Tombstone a doc via the `forget` MCP tool. The server requires `id` and
   * `reason`; the field is `id`, not `doc_id`. Tenant is sent the same way
   * `upsert_note` sends it, so a delete targets the partition the write created.
   */
  async forget(args: { docId: string; reason: string }): Promise<void> {
    const payload = {
      jsonrpc: "2.0",
      id: `obsidian-${Date.now()}`,
      method: "tools/call",
      params: {
        name: "forget",
        arguments: { id: args.docId, reason: args.reason, tenant: this.tenant },
      },
    };

    const response = await this.send({
      url: `${this.base}/mcp`,
      method: "POST",
      headers: this.authHeaders(true),
      body: JSON.stringify(payload),
    });

    const body = this.parseJson(response.text);
    if (body?.error) {
      throw new HarnessError(
        `Harness MCP error: ${body.error.message ?? JSON.stringify(body.error)}`
      );
    }
    const result = body?.result;
    const structured = result?.structuredContent ?? this.firstJsonContent(result);
    if (structured?.error) {
      throw new HarnessError(`forget rejected: ${structured.message ?? structured.error}`);
    }
  }

  /**
   * A lightweight reachability probe for the settings "Test connection" button:
   * hit `/health`, then list the tenant's docs. Returns the health status plus
   * the doc count and a sample title; throws a `HarnessError` on any failure.
   */
  async testConnection(): Promise<{ health: number; count: number; sampleTitle: string }> {
    const health = await this.send({
      url: `${this.base}/health`,
      method: "GET",
      headers: this.authHeaders(false),
    });
    const docs = await this.listDocs("");
    return {
      health: health.status,
      count: docs.count,
      sampleTitle: docs.docs[0]?.title ?? "",
    };
  }

  private async send(params: RequestUrlParam) {
    let response;
    try {
      response = await requestUrl({ ...params, throw: false });
    } catch (error) {
      throw new HarnessError(`Network error reaching the harness: ${String(error)}`);
    }
    if (response.status === 401) {
      throw new HarnessError("Unauthorized: check the bearer token.", 401);
    }
    if (response.status === 403) {
      throw new HarnessError("Forbidden: the token lacks the required scope or tenant.", 403);
    }
    if (response.status >= 400) {
      throw new HarnessError(
        `Harness returned HTTP ${response.status}: ${response.text?.slice(0, 200) ?? ""}`,
        response.status
      );
    }
    return response;
  }

  private parseJson(text: string): any {
    try {
      return JSON.parse(text);
    } catch {
      throw new HarnessError("Harness returned a non-JSON response.");
    }
  }

  /** MCP servers may return JSON in a content[].text block; fall back to parsing it. */
  private firstJsonContent(result: any): any {
    const content = result?.content;
    if (!Array.isArray(content)) {
      return undefined;
    }
    for (const item of content) {
      if (item?.type === "text" && typeof item.text === "string") {
        try {
          return JSON.parse(item.text);
        } catch {
          // keep scanning
        }
      }
    }
    return undefined;
  }
}
