/**
 * Domain types for the Harness Console.
 *
 * These mirror the harness backend contracts the surfaces depend on (memory
 * atoms, skills, rooms, runs, keys, providers, usage). The graph is the source
 * of truth; every type here is a projection of a graph node or edge. Field
 * names track the MCP tool payloads named in the surface spec so the live
 * client can map one-to-one.
 */

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

export type AtomKind =
  | "decision"
  | "feedback"
  | "solution"
  | "postmortem"
  | "preference"
  | "note"
  | "reflection"
  | "handoff"
  | "coordination"
  | "source"
  | "skill";

export type AtomLifecycle = "active" | "archived" | "trash";

export type EdgeKind =
  | "supports"
  | "contradicts"
  | "refines"
  | "derives"
  | "cites"
  | "wikilink"
  | "relates";

export interface MemoryEdge {
  id: string;
  from: string;
  to: string;
  kind: EdgeKind;
}

export interface Atom {
  id: string;
  title: string;
  kind: AtomKind;
  summary: string;
  body: string; // markdown
  /** Slim recall/list payloads leave body empty until getAtom hydrates it. */
  hydrated?: boolean;
  /** Short body preview returned with slim references. */
  contentPreview?: string;
  tags: string[];
  /** salience for recall ranking; fitness for learned/skill atoms. */
  salience: number;
  fitness?: number;
  source?: string;
  created: string; // ISO
  updated: string; // ISO
  lifecycle: AtomLifecycle;
  links: string[]; // wikilink targets, rendered as MEMORY_RELATES edges
  /** UMAP projection coords for the cluster graph; computed server side. */
  x?: number;
  y?: number;
  clusterId?: string;
}

export interface MemoryCluster {
  id: string;
  label: string;
  count: number;
  hue: number; // color seed
}

export interface MemoryList {
  atoms: Atom[];
  edges: MemoryEdge[];
  clusters: MemoryCluster[];
}

export interface MemoryQuery {
  view?: AtomLifecycle;
  kinds?: AtomKind[];
  tags?: string[];
  from?: string;
  to?: string;
  search?: string;
  searchMode?: "fulltext" | "semantic";
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

export type SkillStatus =
  | "draft"
  | "shadow"
  | "advisory"
  | "validated"
  | "canonical"
  | "retired";

export interface SkillFile {
  path: string; // e.g. "SKILL.md" or "scripts/run.py"
  language: "markdown" | "python" | "typescript" | "rust" | "text";
  content: string;
}

export interface Skill {
  id: string;
  name: string;
  description: string; // "what does this skill do"
  status: SkillStatus;
  contentHash: string;
  version: string;
  updated: string;
  files: SkillFile[];
}

export interface SkillUseReceipt {
  skillId: string;
  appliedAt: string;
  ok: boolean;
  summary: string;
  steps: string[];
}

// ---------------------------------------------------------------------------
// Rooms (coordination)
// ---------------------------------------------------------------------------

export type Presence = "live" | "idle" | "away";

export interface RoomParticipant {
  actor: string; // codex, claude-code, claude-ai, human
  presence: Presence;
  lastSeen: string;
}

export type RoomEventKind =
  | "intent"
  | "message"
  | "record"
  | "decision"
  | "tension"
  | "reflection"
  | "mention";

export interface RoomEvent {
  id: string;
  kind: RoomEventKind;
  actor: string;
  text: string;
  at: string;
  mentions?: string[];
}

export interface Room {
  id: string;
  name: string;
  topic: string;
  participants: RoomParticipant[];
  events: RoomEvent[];
  updated: string;
}

// ---------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------

export type RunStatus = "running" | "complete" | "error" | "cancelled";
export type AlignmentVerdict = "aligned" | "flagged" | "blocked" | "pending";

export interface RunStep {
  index: number;
  at: string;
  kind: string; // transition / tool_call / observation / head_contribution
  actor?: string;
  summary: string;
  detail?: string;
}

export interface Run {
  id: string;
  goal: string;
  status: RunStatus;
  verdict: AlignmentVerdict;
  stepCount: number;
  started: string;
  finished?: string;
  steps: RunStep[];
}

// ---------------------------------------------------------------------------
// Agent (composed Theorem agent)
// ---------------------------------------------------------------------------

export interface Head {
  id: string; // e.g. "claude", "deepseek"
  model: string;
  provider: string; // anthropic, deepseek, mistral, openai
  role: string; // proposer, verifier, peer
  keyStatus: "ok" | "missing" | "invalid";
}

export interface AgentScope {
  memoryScopes: string[];
  rooms: string[];
}

export interface AgentBinding {
  bindingId: string; // "agent:theorem"
  heads: Head[];
  scope: AgentScope;
}

export type ChatRole = "user" | "assistant" | "head" | "tool" | "system";

export interface TraceEntry {
  id: string;
  role: ChatRole;
  head?: string;
  content: string;
  at: string;
  tool?: string;
}

export interface ChatMessage {
  id: string;
  role: ChatRole;
  content: string;
  at: string;
  head?: string;
  trace?: TraceEntry[];
  verdict?: AlignmentVerdict;
}

// ---------------------------------------------------------------------------
// Keys (inbound), Providers (outbound), Usage
// ---------------------------------------------------------------------------

export type ClientKind = "claude" | "codex" | "gemini" | "raw";

export interface HarnessKey {
  id: string;
  name: string;
  prefix: string;
  created: string;
  lastUsed?: string;
  scopes: string[];
}

export type ProviderName = "anthropic" | "deepseek" | "mistral" | "openai";

export interface Provider {
  name: ProviderName;
  label: string;
  keyStatus: "ok" | "missing" | "invalid";
  defaultModel: string;
  mode: "byok" | "credits"; // bring-your-own-key vs harness credits
  credentialRef?: string;
}

export interface UsagePeriod {
  requests: number;
  toolCalls: number;
  limit: number;
  plan: string;
  payPerUse: boolean;
  periodLabel: string;
  series: { label: string; value: number }[];
}

// ---------------------------------------------------------------------------
// Connections + MCP Hub
// ---------------------------------------------------------------------------

export type IngestStatus = "queued" | "ingesting" | "indexed" | "error";

export interface ConnectedRepo {
  id: string;
  owner: string;
  name: string;
  status: IngestStatus;
  symbols?: number;
  updated: string;
}

export interface GithubConnection {
  connected: boolean;
  account?: string;
  repos: ConnectedRepo[];
}

export interface CapabilityNamespace {
  id: string;
  label: string;
  description: string;
  verbs: number;
  enabled: boolean;
}

export interface BrokeredServer {
  id: string;
  name: string;
  transport: "http" | "stdio";
  url?: string;
  status: "connected" | "error" | "disabled";
  tools: number;
}

export interface McpHubState {
  namespaces: CapabilityNamespace[];
  brokered: BrokeredServer[];
}

// ---------------------------------------------------------------------------
// Onboarding
// ---------------------------------------------------------------------------

export interface RegisterResult {
  key: string;
  prefix: string;
  tenant: string;
  claimUrl: string;
  expiresAt: string;
}

// ---------------------------------------------------------------------------
// Search (shared by Memory + omnibar + Dynamic Island)
// ---------------------------------------------------------------------------

export type SearchResultKind = "atom" | "room" | "run" | "skill" | "action" | "web";

export interface SearchResult {
  id: string;
  kind: SearchResultKind;
  title: string;
  subtitle?: string;
  score?: number;
  href?: string;
}

// ---------------------------------------------------------------------------
// Inbox + Tasks (the action queue: mentions/runs/system + Dispatch-v2 jobs)
// ---------------------------------------------------------------------------

export type InboxKind = "mention" | "run" | "system" | "job";

export interface InboxItem {
  id: string;
  kind: InboxKind;
  title: string;
  from: string; // actor or system source
  preview: string;
  body: string;
  at: string;
  read: boolean;
  room?: string;
  href?: string; // where the action lives (a run, a room, a job)
}

export type TaskState = "queued" | "running" | "done" | "blocked";
export type TaskPriority = "low" | "normal" | "high";

export interface Task {
  id: string;
  title: string;
  state: TaskState;
  priority: TaskPriority;
  targetHead?: string; // claude / codex / gemini
  updated: string;
  note?: string;
  runId?: string;
}
