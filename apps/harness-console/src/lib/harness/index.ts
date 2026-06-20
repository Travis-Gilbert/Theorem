/**
 * Single entry point for the harness client. Surfaces import `harness` from
 * here and never touch a transport directly.
 */
import { type HarnessClient, HARNESS_SOURCE } from "./client";
import { mockClient } from "./mock";
import { liveClient } from "./mcp";

export const harness: HarnessClient = HARNESS_SOURCE === "live" ? liveClient : mockClient;

export * from "./types";
export { HARNESS_URL, HARNESS_MCP_PATH, HARNESS_SOURCE, installSnippet } from "./client";
