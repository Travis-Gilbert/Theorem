// Shared domain types for the Theorem Desktop shell (phase one, the Dia rebuild).
// These types are the vocabulary of both the frontend state (state/store.tsx)
// and the invoke() command contract (lib/commands.ts). Keep them framework-free.

export type TabId = string;
export type SpaceId = string;

export type TabKind = "web" | "newtab" | "agent";

/** A single tab. One web tab maps to one wry webview owned by the Rust backend. */
export interface Tab {
  id: TabId;
  kind: TabKind;
  /** Live URL for web tabs; empty for the ask-first new-tab page. */
  url: string;
  title: string;
  favicon?: string;
  pinned: boolean;
  /** Space membership; undefined means the default (ungrouped) space. */
  spaceId?: SpaceId;
  /** Phase five: agent tabs feed captured page text to web_consume. */
  agentIngestionEnabled?: boolean;
  /** Backend-reported navigation state, mirrored into the chrome. */
  loading?: boolean;
  canGoBack?: boolean;
  canGoForward?: boolean;
}

/** A named tab group. "Spaces" in the Dia anatomy. */
export interface Space {
  id: SpaceId;
  name: string;
  order: number;
  /** Phase four: optional coordination room binding. */
  roomId?: string;
}

export type ChatRole = "user" | "assistant" | "system";

/** A reference to another open tab pulled into a rail turn via @-mention. */
export interface MentionRef {
  tabId: TabId;
  title: string;
  url: string;
}

/** Extracted page context that attaches to a rail turn (D4). */
export interface PageContext {
  url: string;
  title: string;
  /** Current text selection in the page, if any. */
  selection?: string;
  /** Visible text extracted from the webview via injected JS (backend). */
  text?: string;
}

export interface ChatTurn {
  id: string;
  role: ChatRole;
  text: string;
  /** Tabs the user @-mentioned for this turn (the signature interaction). */
  mentions?: MentionRef[];
  /** The active tab's context captured at send time. */
  pageContext?: PageContext;
  createdAt: number;
  /** True while an assistant turn is still streaming/pending. */
  pending?: boolean;
  /** Phase six: visible per-turn estimated usage/cost. */
  usage?: TurnUsage;
}

export interface TurnUsage {
  provider: ProviderId;
  model: string;
  tokensIn: number;
  tokensOut: number;
  estimatedUsd: number;
}

/** The conversation bound to a single tab. The rail shows the active tab's. */
export interface Conversation {
  tabId: TabId;
  turns: ChatTurn[];
}

/** A prior-memory hit surfaced in the known-context strip (D4). Text only. */
export interface RecallHit {
  id: string;
  title: string;
  snippet: string;
  tags?: string[];
  url?: string;
  createdAt?: number;
}

export type ProviderId = "anthropic" | "openai" | "deepseek" | "ollama" | "local";
export type HarnessTarget = "hosted" | "local";

/** Provider roster. DeepSeek is the keyless default per the standing decision. */
export const PROVIDERS: { id: ProviderId; label: string; keyless: boolean }[] = [
  { id: "deepseek", label: "DeepSeek", keyless: true },
  { id: "local", label: "Local", keyless: true },
  { id: "ollama", label: "Ollama", keyless: true },
  { id: "anthropic", label: "Anthropic", keyless: false },
  { id: "openai", label: "OpenAI", keyless: false },
];

/** Harness connection settings (D5). The bearer token lives in the keychain. */
export interface HarnessSettings {
  endpoint: string;
  /** Loopback MCP endpoint for the phase-two local node. */
  localEndpoint: string;
  /** Which harness target new memory calls should use. */
  activeTarget: HarnessTarget;
  tenant: string;
  /** Whether a bearer token is stored in the keychain (never the token itself). */
  bearerPresent: boolean;
}

export interface ReceiverSettings {
  enabled: boolean;
  claimIntervalSecs: number;
  worktrees: Record<string, string>;
}

export interface SyncSettings {
  enabled: boolean;
  intervalSecs: number;
}

export interface BackgroundFetchSettings {
  enabled: boolean;
  intervalSecs: number;
}

export interface OllamaSettings {
  endpoint: string;
  model: string;
}

export interface SyncReceipt {
  id: string;
  status: "disabled" | "ok" | "error";
  startedAt: string;
  finishedAt?: string;
  localPack?: string;
  hostedPack?: string;
  mergedNodes?: number;
  mergedEdges?: number;
  conflicts?: number;
  message: string;
}

export interface RoomFeedItem {
  id: string;
  actor: string;
  text: string;
  createdAt?: string;
  kind?: string;
}

export interface RoomParticipant {
  actor: string;
  status: string;
  lastSeen?: string;
}

export interface RoomIntent {
  actor: string;
  status: string;
  summary: string;
  footprint: string[];
  updatedAt?: string;
  expectedCompletion?: string;
  repo?: string;
  branch?: string;
  task?: string;
}

export interface RoomRecord {
  id: string;
  kind: string;
  actor?: string;
  title?: string;
  summary: string;
  body?: string;
  refs: string[];
  createdAt?: string;
}

export interface QueueJob {
  jobId: string;
  title: string;
  status: string;
  targetHead?: string;
  priority?: string;
  age?: string;
}

export interface AgentIngestionReceipt {
  id: string;
  status: "disabled" | "ok" | "error";
  url: string;
  title?: string;
  capturedAt: string;
  storeTarget: HarnessTarget;
  trustTier: "open_web_unverified";
  message: string;
}

export interface CostSummary {
  turns: number;
  tokensIn: number;
  tokensOut: number;
  estimatedUsd: number;
}

/** The slice of state that persists to SQLite and restores on launch (D3). */
export interface SessionState {
  tabs: Tab[];
  spaces: Space[];
  activeTabId: TabId | null;
}

/** What the settings surface manages (D5). Secrets are presence-flags only. */
export interface Settings {
  harness: HarnessSettings;
  receiver: ReceiverSettings;
  sync: SyncSettings;
  backgroundFetch: BackgroundFetchSettings;
  ollama: OllamaSettings;
  defaultModel: ProviderId;
  /** Which providers have a key stored in the OS keychain. Never the keys. */
  providerKeyPresent: Record<ProviderId, boolean>;
}

/** Full in-memory app state. SessionState is the persisted subset. */
export interface AppState {
  tabs: Tab[];
  spaces: Space[];
  activeTabId: TabId | null;
  conversations: Record<TabId, Conversation>;
  recallByDomain: Record<string, RecallHit[]>;
  roomFeedBySpace: Record<SpaceId, RoomFeedItem[]>;
  participantsBySpace: Record<SpaceId, RoomParticipant[]>;
  roomIntentsBySpace: Record<SpaceId, RoomIntent[]>;
  roomRecordsBySpace: Record<SpaceId, RoomRecord[]>;
  queueJobs: QueueJob[];
  syncReceipts: SyncReceipt[];
  agentIngestionReceipts: AgentIngestionReceipt[];
  costSummary: CostSummary;
  railVisible: boolean;
  railView: "chat" | "room";
  queuePanelOpen: boolean;
  settingsOpen: boolean;
  settings: Settings;
}
