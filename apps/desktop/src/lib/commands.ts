// The invoke() command contract: the CC/Codex seam for Theorem Desktop phase one.
//
// Claude Code authors this file as the agreed interface. Codex implements the
// matching #[tauri::command] handlers in apps/desktop/src-tauri. The Rust
// command name is given in a comment above each wrapper; argument keys must
// match exactly (Tauri maps camelCase TS keys to snake_case Rust params, so we
// pass the keys Tauri expects).
//
// Every wrapper degrades gracefully when not running inside Tauri (plain Vite
// browser mode): it returns honest in-memory mock data so the frontend shell
// renders and the omnibox/sidebar/rail are exercisable before the Rust backend
// lands. The real path is always invoke(); mocks are gated behind isTauri().

import { invoke } from "@tauri-apps/api/core";
import type {
  HarnessTarget,
  AgentIngestionReceipt,
  QueueJob,
  RoomFeedItem,
  RoomParticipant,
  HarnessSettings,
  PageContext,
  ProviderId,
  RecallHit,
  ReceiverSettings,
  SessionState,
  SyncReceipt,
  TabId,
  TurnUsage,
} from "../state/types";

/** True when running inside the Tauri runtime (vs. a plain browser dev server). */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

// --- Tab / webview lifecycle (D3) -- Rust: src-tauri, one wry webview per tab.
//
// Identity ownership: the FRONTEND owns tab identity (crypto.randomUUID). The
// backend manages wry webviews keyed by that TabId. The ask-first new-tab page
// is pure DOM with no webview -- for it, call tabSetActive(null) so the backend
// hides all webviews and the React new-tab page shows through.

/**
 * Rust: `tab_create(tab_id: String, url: Option<String>)`. Creates a wry webview
 * bound to this frontend-owned TabId; if url is given, navigates to it.
 */
export async function tabCreate(tabId: TabId, url?: string): Promise<void> {
  if (isTauri()) return invoke("tab_create", { tabId, url: url ?? null });
}

/** Rust: `tab_navigate(tab_id: String, url: String)`. */
export async function tabNavigate(tabId: TabId, url: string): Promise<void> {
  if (isTauri()) return invoke("tab_navigate", { tabId, url });
}

/** Rust: `tab_reload(tab_id: String)`. */
export async function tabReload(tabId: TabId): Promise<void> {
  if (isTauri()) return invoke("tab_reload", { tabId });
}

/** Rust: `tab_go_back(tab_id: String)`. */
export async function tabGoBack(tabId: TabId): Promise<void> {
  if (isTauri()) return invoke("tab_go_back", { tabId });
}

/** Rust: `tab_go_forward(tab_id: String)`. */
export async function tabGoForward(tabId: TabId): Promise<void> {
  if (isTauri()) return invoke("tab_go_forward", { tabId });
}

/** Rust: `tab_close(tab_id: String)`. */
export async function tabClose(tabId: TabId): Promise<void> {
  if (isTauri()) return invoke("tab_close", { tabId });
}

/**
 * Rust: `tab_set_active(tab_id: Option<String>)`. Shows that tab's webview and
 * hides the others; null means the new-tab page is showing (no webview).
 */
export async function tabSetActive(tabId: TabId | null): Promise<void> {
  if (isTauri()) return invoke("tab_set_active", { tabId });
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Rust: `tab_set_bounds(rect: Rect)`. Positions the active webview to fill the
 * stage hole left by the chrome (sidebar + omnibox + optional rail).
 */
export async function tabSetBounds(rect: Rect): Promise<void> {
  if (isTauri()) return invoke("tab_set_bounds", { rect });
}

// --- Page extraction (D4) -- Rust injects JS into the webview, text only.

/** Rust: `extract_visible_text(tab_id: String) -> PageContext`. */
export async function extractVisibleText(tabId: TabId): Promise<PageContext> {
  if (isTauri()) return invoke<PageContext>("extract_visible_text", { tabId });
  return {
    url: "https://example.com",
    title: "Example Domain",
    text: "Example Domain. This domain is for use in illustrative examples.",
  };
}

// --- Session persistence (D3) -- Rust: tauri-plugin-sql (SQLite).

/** Rust: `session_load() -> Option<SessionState>`. */
export async function sessionLoad(): Promise<SessionState | null> {
  if (isTauri()) return invoke<SessionState | null>("session_load");
  return null;
}

/** Rust: `session_save(state: SessionState)`. */
export async function sessionSave(state: SessionState): Promise<void> {
  if (isTauri()) return invoke("session_save", { state });
}

// --- Keychain (D5) -- Rust: OS keychain plugin. Keys never leave the backend.

/** Rust: `keychain_set(provider: String, key: String)`. */
export async function keychainSet(provider: ProviderId, key: string): Promise<void> {
  if (isTauri()) return invoke("keychain_set", { provider, key });
}

/** Rust: `keychain_has(provider: String) -> bool`. Never returns the key. */
export async function keychainHas(provider: ProviderId): Promise<boolean> {
  if (isTauri()) return invoke<boolean>("keychain_has", { provider });
  return false;
}

/** Rust: `keychain_delete(provider: String)`. */
export async function keychainDelete(provider: ProviderId): Promise<void> {
  if (isTauri()) return invoke("keychain_delete", { provider });
}

// --- Harness memory (D4) -- Rust: hosted MCP client, bearer + tenant.

export interface RememberInput {
  text: string;
  /** Page provenance for the turn (url/title). */
  url?: string;
  title?: string;
  tags?: string[];
  /** Free-form provenance, e.g. mentioned tab urls. */
  provenance?: Record<string, unknown>;
}

/** Rust: `harness_remember(input) -> { id, tags }`. */
export async function harnessRemember(
  input: RememberInput,
): Promise<{ id: string; tags: string[] }> {
  if (isTauri()) return invoke("harness_remember", { input });
  return { id: `mock-mem-${Math.round(performance.now())}`, tags: input.tags ?? [] };
}

export interface RecallQuery {
  text?: string;
  domain?: string;
  limit?: number;
}

/** Rust: `harness_recall(query) -> RecallHit[]`. Powers the known-context strip. */
export async function harnessRecall(query: RecallQuery): Promise<RecallHit[]> {
  if (isTauri()) return invoke<RecallHit[]>("harness_recall", { query });
  return [];
}

/** Rust: `harness_settings_get() -> HarnessSettings`. */
export async function harnessSettingsGet(): Promise<HarnessSettings | null> {
  if (isTauri()) return invoke<HarnessSettings | null>("harness_settings_get");
  return null;
}

/** Rust: `harness_settings_set(settings: HarnessSettings)`. */
export async function harnessSettingsSet(settings: HarnessSettings): Promise<void> {
  if (isTauri()) return invoke("harness_settings_set", { settings });
}

export interface LocalNodeStatus {
  nodeUp: boolean;
  endpoint: string;
  port: number;
  storePath: string;
  activeTarget: HarnessTarget;
  toolsMatchHosted: boolean;
}

export interface ReceiverStatus {
  enabled: boolean;
  state: "off" | "configured" | "running" | "error";
  lanes: string[];
  lastClaimTime?: string;
  lastJobResult?: string;
}

/** Rust: `local_node_status() -> LocalNodeStatus`. */
export async function localNodeStatus(): Promise<LocalNodeStatus> {
  if (isTauri()) return invoke<LocalNodeStatus>("local_node_status");
  return {
    nodeUp: false,
    endpoint: "http://127.0.0.1:17888/mcp",
    port: 17888,
    storePath: "~/Library/Application Support/Theorem/store",
    activeTarget: "hosted",
    toolsMatchHosted: false,
  };
}

/** Rust: `receiver_settings_get() -> ReceiverSettings`. */
export async function receiverSettingsGet(): Promise<ReceiverSettings | null> {
  if (isTauri()) return invoke<ReceiverSettings | null>("receiver_settings_get");
  return null;
}

/** Rust: `receiver_settings_set(settings: ReceiverSettings)`. */
export async function receiverSettingsSet(settings: ReceiverSettings): Promise<void> {
  if (isTauri()) return invoke("receiver_settings_set", { settings });
}

/** Rust: `receiver_status() -> ReceiverStatus`. */
export async function receiverStatus(): Promise<ReceiverStatus> {
  if (isTauri()) return invoke<ReceiverStatus>("receiver_status");
  return {
    enabled: false,
    state: "off",
    lanes: [],
  };
}

/** Rust: `harness_bearer_set(token: String)`. Bearer is a secret -> keychain. */
export async function harnessBearerSet(token: string): Promise<void> {
  if (isTauri()) return invoke("harness_bearer_set", { token });
}

/** Rust: `harness_bearer_clear()`. */
export async function harnessBearerClear(): Promise<void> {
  if (isTauri()) return invoke("harness_bearer_clear");
}

// --- Model chat (D4) -- Rust: BYO provider keys, DeepSeek keyless default.

export interface ModelMessage {
  role: "user" | "assistant" | "system";
  content: string;
}

export interface ModelChatInput {
  model: ProviderId;
  messages: ModelMessage[];
  ollamaEndpoint?: string;
  ollamaModel?: string;
}

export interface ModelChatResult {
  content: string;
  usage?: TurnUsage;
}

/** Rust: `model_chat(input) -> ModelChatResult`. Streaming is a later addition. */
export async function modelChat(input: ModelChatInput): Promise<ModelChatResult> {
  if (isTauri()) return invoke<ModelChatResult>("model_chat", { input });
  // Web-mode placeholder so the rail is exercisable without a provider key.
  const last = input.messages[input.messages.length - 1];
  return {
    content:
      `[dev placeholder, no backend] You said: "${last?.content ?? ""}". ` +
      `The real answer will come from ${input.model} via the Rust model-client.`,
    usage: {
      provider: input.model,
      model: input.model === "ollama" ? "local-dev" : input.model,
      tokensIn: Math.ceil(input.messages.map((m) => m.content).join(" ").length / 4),
      tokensOut: Math.ceil((last?.content ?? "").length / 5),
      estimatedUsd: 0,
    },
  };
}

// --- Sync (phase three) -----------------------------------------------------

/** Rust: `sync_run() -> SyncReceipt`. Executes one sync round when enabled. */
export async function syncRun(): Promise<SyncReceipt> {
  if (isTauri()) return invoke<SyncReceipt>("sync_run");
  return {
    id: `mock-sync-${Math.round(performance.now())}`,
    status: "disabled",
    startedAt: new Date().toISOString(),
    message: "Sync disabled in browser preview.",
  };
}

export interface BackgroundFetchInput {
  urls: string[];
}

/** Rust: `background_fetch_receipt(input)`. Records the background fetch pass. */
export async function backgroundFetchReceipt(input: BackgroundFetchInput): Promise<void> {
  if (isTauri()) return invoke("background_fetch_receipt", { input });
}

// --- Agent spaces and queue (phase four) ------------------------------------

export interface SpaceBindInput {
  roomId: string;
  spaceName: string;
}

/** Rust: `space_bind_room(input)`. Starts/joins a room for a Space. */
export async function spaceBindRoom(input: SpaceBindInput): Promise<void> {
  if (isTauri()) return invoke("space_bind_room", { input });
}

export interface RoomContext {
  feed: RoomFeedItem[];
  participants: RoomParticipant[];
}

/** Rust: `room_context(room_id) -> RoomContext`. */
export async function roomContext(roomId: string): Promise<RoomContext> {
  if (isTauri()) return invoke<RoomContext>("room_context", { roomId });
  return { feed: [], participants: [] };
}

export interface RoomPostInput {
  roomId: string;
  message: string;
}

/** Rust: `room_post_message(input)`. */
export async function roomPostMessage(input: RoomPostInput): Promise<void> {
  if (isTauri()) return invoke("room_post_message", { input });
}

export interface JobSubmitInput {
  title: string;
  specRef: string;
  repo: string;
  kind: "ImplementSpec" | "Feature" | "Edit" | "App" | "Investigation";
  priority?: "P0" | "P1" | "P2";
  targetHead?: "ClaudeCode" | "Codex" | "Either";
}

/** Rust: `job_submit(input)`. */
export async function jobSubmit(input: JobSubmitInput): Promise<void> {
  if (isTauri()) return invoke("job_submit", { input });
}

export interface QueueStatusInput {
  repo?: string;
  status?: string;
}

/** Rust: `queue_status(input) -> QueueJob[]`. */
export async function queueStatus(input: QueueStatusInput = {}): Promise<QueueJob[]> {
  if (isTauri()) return invoke<QueueJob[]>("queue_status", { input });
  return [];
}

// --- Agent tab ingestion (phase five) ---------------------------------------

export interface AgentTabIngestInput {
  tabId: TabId;
  url: string;
  title?: string;
  text: string;
}

/** Rust: `agent_tab_ingest(input) -> AgentIngestionReceipt`. */
export async function agentTabIngest(
  input: AgentTabIngestInput,
): Promise<AgentIngestionReceipt> {
  if (isTauri()) return invoke<AgentIngestionReceipt>("agent_tab_ingest", { input });
  return {
    id: `mock-ingest-${Math.round(performance.now())}`,
    status: "disabled",
    url: input.url,
    title: input.title,
    capturedAt: new Date().toISOString(),
    storeTarget: "hosted",
    trustTier: "open_web_unverified",
    message: "Agent ingestion disabled in browser preview.",
  };
}

// --- Integrations proof (phase six) -----------------------------------------

export interface ConnectorProofResult {
  status: "ok" | "error";
  affordanceId: string;
  message: string;
}

/** Rust: `connector_proof_run() -> ConnectorProofResult`. */
export async function connectorProofRun(): Promise<ConnectorProofResult> {
  if (isTauri()) return invoke<ConnectorProofResult>("connector_proof_run");
  return {
    status: "error",
    affordanceId: "theorem_grpc.code_search.search",
    message: "Connector proof runs only in the desktop backend.",
  };
}
