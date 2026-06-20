/**
 * The typed harness client contract.
 *
 * Every surface depends only on this interface, never on a transport. Two
 * implementations satisfy it: `mock` (local fixtures, the default) and `live`
 * (real MCP/HTTP). `getClient()` selects by NEXT_PUBLIC_HARNESS_SOURCE so a
 * single env flip moves the whole console onto the real backend.
 */
import type {
  Atom,
  MemoryList,
  MemoryQuery,
  Skill,
  SkillUseReceipt,
  Room,
  Run,
  AgentBinding,
  ChatMessage,
  HarnessKey,
  Provider,
  UsagePeriod,
  GithubConnection,
  McpHubState,
  RegisterResult,
  SearchResult,
  ClientKind,
  InboxItem,
  Task,
  TaskState,
} from "./types";

export interface HarnessClient {
  // Memory
  listMemory(query?: MemoryQuery): Promise<MemoryList>;
  getAtom(id: string): Promise<Atom | null>;
  saveAtom(atom: Atom): Promise<Atom>; // self_revise / upsert_note
  archiveAtom(id: string): Promise<void>; // self_archive
  trashAtom(id: string, reason: string): Promise<void>; // forget
  restoreAtom(id: string): Promise<void>;
  search(q: string, mode: "fulltext" | "semantic"): Promise<SearchResult[]>;

  // Skills
  listSkills(): Promise<Skill[]>;
  getSkill(id: string): Promise<Skill | null>;
  publishSkill(skill: Skill): Promise<Skill>; // skill_publish, bumps hash
  applySkill(id: string): Promise<SkillUseReceipt>; // skill_apply

  // Rooms + Runs (read-only windows)
  listRooms(): Promise<Room[]>;
  getRoom(id: string): Promise<Room | null>;
  listRuns(): Promise<Run[]>;
  getRun(id: string): Promise<Run | null>;

  // Agent
  getBinding(): Promise<AgentBinding>;
  runAgent(prompt: string, scope?: string[]): Promise<ChatMessage>; // composed_agent_run

  // Keys (inbound) + onboarding
  listKeys(): Promise<HarnessKey[]>;
  createKey(name: string, scopes: string[]): Promise<HarnessKey>;
  revokeKey(id: string): Promise<void>;
  registerAnonymous(): Promise<RegisterResult>;
  installSnippet(client: ClientKind, keyPrefix: string): string;

  // Providers (outbound)
  listProviders(): Promise<Provider[]>;
  validateProvider(name: string): Promise<{ ok: boolean; message: string }>;

  // Usage
  getUsage(): Promise<UsagePeriod>;

  // Connections + MCP hub
  getGithub(): Promise<GithubConnection>;
  getMcpHub(): Promise<McpHubState>;
  toggleNamespace(id: string, enabled: boolean): Promise<void>;

  // Inbox + tasks (the action queue)
  listInbox(): Promise<InboxItem[]>;
  markInboxRead(id: string, read?: boolean): Promise<void>;
  archiveInboxItem(id: string): Promise<void>;
  listTasks(): Promise<Task[]>;
  updateTaskState(id: string, state: TaskState): Promise<void>;
}

export const HARNESS_URL =
  process.env.NEXT_PUBLIC_HARNESS_URL ??
  "https://rustyredcore-theorem-production.up.railway.app";
export const HARNESS_MCP_PATH = process.env.NEXT_PUBLIC_HARNESS_MCP_PATH ?? "/mcp";
export const HARNESS_SOURCE = process.env.NEXT_PUBLIC_HARNESS_SOURCE ?? "mock";

/**
 * Per-client install blocks. The tenant is baked into the key and resolved
 * server side, so the only things a user pastes are the URL and the key.
 */
export function installSnippet(client: ClientKind, key: string): string {
  const url = `${HARNESS_URL}${HARNESS_MCP_PATH}`;
  switch (client) {
    case "claude":
      return `claude mcp add --transport http harness \\\n  ${url} \\\n  --header "Authorization: Bearer ${key}"`;
    case "codex":
      return `# ~/.codex/config.toml\n[mcp_servers.harness]\nurl = "${url}"\nbearer_token_env_var = "HARNESS_API_KEY"\n\n# then, in your shell:\nexport HARNESS_API_KEY=${key}`;
    case "gemini":
      return `// ~/.gemini/settings.json\n{\n  "mcpServers": {\n    "harness": {\n      "httpUrl": "${url}",\n      "headers": { "Authorization": "Bearer ${key}" }\n    }\n  }\n}`;
    case "raw":
    default:
      return `POST ${url}\nAuthorization: Bearer ${key}\nContent-Type: application/json\n\n{ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }`;
  }
}
