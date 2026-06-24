/**
 * Mock implementation of HarnessClient backed by deterministic fixtures.
 *
 * In-memory mutations persist for the session so the surfaces feel live (edit
 * an atom, archive it, publish a skill, create a key) without a backend. State
 * is module-scoped, so it resets on reload, which is correct for a demo source.
 */
import { type HarnessClient, installSnippet } from "./client";
import type {
  Atom,
  MemoryList,
  MemoryQuery,
  Skill,
  SkillUseReceipt,
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
} from "./types";
import {
  ATOMS,
  EDGES,
  CLUSTERS,
  SKILLS,
  ROOMS,
  RUNS,
  BINDING,
  KEYS,
  PROVIDERS,
  USAGE,
  GITHUB,
  MCP_HUB,
  INBOX,
  TASKS,
} from "./fixtures";
import type { InboxItem, Task, TaskState } from "./types";

// Mutable session state seeded from fixtures.
let atoms = [...ATOMS];
const skills = [...SKILLS];
const keys = [...KEYS];
const hub: McpHubState = { namespaces: MCP_HUB.namespaces.map((n) => ({ ...n })), brokered: MCP_HUB.brokered };
let inbox: InboxItem[] = INBOX.map((i) => ({ ...i }));
const tasks: Task[] = TASKS.map((t) => ({ ...t }));

const delay = <T,>(value: T, ms = 90): Promise<T> =>
  new Promise((resolve) => setTimeout(() => resolve(value), ms));

function contentPreview(body: string, chars = 220): string {
  return body.replace(/\s+/g, " ").trim().slice(0, chars);
}

function slimAtom(atom: Atom): Atom {
  return {
    ...atom,
    body: "",
    hydrated: false,
    contentPreview: contentPreview(atom.body),
  };
}

function hydratedAtom(atom: Atom): Atom {
  return {
    ...atom,
    hydrated: true,
    contentPreview: contentPreview(atom.body),
  };
}

function matchesQuery(a: Atom, q?: MemoryQuery): boolean {
  if (!q) return a.lifecycle === "active";
  if ((q.view ?? "active") !== a.lifecycle) return false;
  if (q.kinds && q.kinds.length && !q.kinds.includes(a.kind)) return false;
  if (q.tags && q.tags.length && !q.tags.some((t) => a.tags.includes(t))) return false;
  if (q.from && new Date(a.updated) < new Date(q.from)) return false;
  if (q.to && new Date(a.updated) > new Date(q.to)) return false;
  if (q.search) {
    const hay = `${a.title} ${a.summary} ${a.tags.join(" ")}`.toLowerCase();
    if (!hay.includes(q.search.toLowerCase())) return false;
  }
  return true;
}

export const mockClient: HarnessClient = {
  async listMemory(query) {
    const filtered = atoms.filter((a) => matchesQuery(a, query));
    const ids = new Set(filtered.map((a) => a.id));
    return delay<MemoryList>({
      atoms: filtered.map(slimAtom),
      edges: EDGES.filter((e) => ids.has(e.from) && ids.has(e.to)),
      clusters: CLUSTERS,
    });
  },
  async getAtom(id) {
    const atom = atoms.find((a) => a.id === id);
    return delay(atom ? hydratedAtom(atom) : null);
  },
  async saveAtom(atom) {
    const next: Atom = { ...atom, updated: new Date().toISOString(), hydrated: true };
    atoms = atoms.map((a) => (a.id === atom.id ? next : a));
    if (!atoms.find((a) => a.id === atom.id)) atoms.push(next);
    return delay(hydratedAtom(next));
  },
  async archiveAtom(id) {
    atoms = atoms.map((a) => (a.id === id ? { ...a, lifecycle: "archived" } : a));
    return delay(undefined);
  },
  async trashAtom(id) {
    atoms = atoms.map((a) => (a.id === id ? { ...a, lifecycle: "trash" } : a));
    return delay(undefined);
  },
  async restoreAtom(id) {
    atoms = atoms.map((a) => (a.id === id ? { ...a, lifecycle: "active" } : a));
    return delay(undefined);
  },
  async search(q, mode) {
    const term = q.toLowerCase();
    const hits: SearchResult[] = atoms
      .filter((a) => a.lifecycle === "active" && `${a.title} ${a.summary}`.toLowerCase().includes(term))
      .slice(0, 12)
      .map((a, i) => ({
        id: a.id,
        kind: "atom",
        title: a.title,
        subtitle: a.summary,
        score: mode === "semantic" ? 0.92 - i * 0.04 : undefined,
        href: `/memory?atom=${a.id}`,
      }));
    return delay(hits);
  },

  async listSkills() {
    return delay(skills);
  },
  async getSkill(id) {
    return delay(skills.find((s) => s.id === id) ?? null);
  },
  async publishSkill(skill) {
    const hash = `sha256:${Math.abs(skill.files.reduce((h, f) => h + f.content.length, 0)).toString(16)}...pub`;
    const idx = skills.findIndex((s) => s.id === skill.id);
    const next: Skill = { ...skill, contentHash: hash, updated: new Date().toISOString() };
    if (idx >= 0) skills[idx] = next;
    else skills.push(next);
    return delay(next);
  },
  async applySkill(id) {
    const s = skills.find((x) => x.id === id);
    return delay<SkillUseReceipt>({
      skillId: id,
      appliedAt: new Date().toISOString(),
      ok: true,
      summary: `Applied ${s?.name ?? id}; use receipt recorded.`,
      steps: ["resolve pack", "compile toolkit", "run", "record receipt"],
    });
  },

  async listRooms() {
    return delay(ROOMS);
  },
  async getRoom(id) {
    return delay(ROOMS.find((r) => r.id === id) ?? null);
  },
  async listRuns() {
    return delay(RUNS);
  },
  async getRun(id) {
    return delay(RUNS.find((r) => r.id === id) ?? null);
  },

  async getBinding() {
    return delay<AgentBinding>(BINDING);
  },
  async runAgent(prompt) {
    const now = new Date().toISOString();
    const lower = prompt.toLowerCase();
    const trace: NonNullable<ChatMessage["trace"]> = [
      {
        id: "t0",
        role: "tool" as const,
        tool: "recall",
        content: "reasoning-strategy recall returned slim references; hydrated only the top relevant strategy",
        at: now,
      },
    ];
    if (/cite|cited|evidence|claim|source/.test(lower)) {
      trace.push({
        id: "t1",
        role: "tool" as const,
        tool: "rustyred_thg_symbolic_probabilistic_source_reliability",
        content: "governor injected source-reliability before answer generation",
        at: now,
      });
    }
    if (/verify|deep retrieval|large recall|adversarial/.test(lower)) {
      trace.push({
        id: "t2",
        role: "tool" as const,
        tool: "rustyred_thg_symbolic_probabilistic_expected_value",
        content: "governor gated the costly check with expected value of information",
        at: now,
      });
    }
    trace.push(
      { id: "t3", role: "head" as const, head: "claude", content: "answer with injected substrate context", at: now },
      { id: "t4", role: "head" as const, head: "deepseek", content: "verify plan against run ledger", at: now },
      { id: "t5", role: "system" as const, content: "alignment-gate: aligned", at: now },
    );
    const msg: ChatMessage = {
      id: `msg_${Date.now()}`,
      role: "assistant",
      content: `The composed agent ran with proactive governor context and slim-first strategy memory. Answer to: "${prompt}".`,
      at: now,
      verdict: "aligned",
      trace,
    };
    return delay(msg, 400);
  },

  async listKeys() {
    return delay(keys);
  },
  async createKey(name, scopes) {
    const k: HarnessKey = {
      id: `key_${Date.now()}`,
      name,
      prefix: `hk_live_${Math.abs(name.length * 7919).toString(16).slice(0, 6)}`,
      created: new Date().toISOString(),
      scopes,
    };
    keys.unshift(k);
    return delay(k);
  },
  async revokeKey(id) {
    const idx = keys.findIndex((k) => k.id === id);
    if (idx >= 0) keys.splice(idx, 1);
    return delay(undefined);
  },
  async registerAnonymous() {
    const tenant = `anon-${Math.abs(Date.now() % 99991).toString(16)}`;
    return delay<RegisterResult>({
      key: `hk_live_${tenant}9f3a2b1c8d`,
      prefix: `hk_live_${tenant}`,
      tenant,
      claimUrl: `https://harness.theoremsweb.com/claim?t=${tenant}`,
      expiresAt: new Date(Date.now() + 1000 * 60 * 60 * 24).toISOString(),
    });
  },
  installSnippet(client: ClientKind, prefix: string) {
    return installSnippet(client, prefix);
  },

  async listProviders() {
    return delay<Provider[]>(PROVIDERS);
  },
  async validateProvider(name) {
    const p = PROVIDERS.find((x) => x.name === name);
    const ok = p?.keyStatus === "ok";
    return delay({ ok, message: ok ? `${name} key is valid.` : `${name} key is ${p?.keyStatus ?? "missing"}.` });
  },

  async getUsage() {
    return delay<UsagePeriod>(USAGE);
  },

  async getGithub() {
    return delay<GithubConnection>(GITHUB);
  },
  async getMcpHub() {
    return delay(hub);
  },
  async toggleNamespace(id, enabled) {
    const ns = hub.namespaces.find((n) => n.id === id);
    if (ns) ns.enabled = enabled;
    return delay(undefined);
  },

  async listInbox() {
    return delay(inbox);
  },
  async markInboxRead(id, read = true) {
    inbox = inbox.map((i) => (i.id === id ? { ...i, read } : i));
    return delay(undefined);
  },
  async archiveInboxItem(id) {
    inbox = inbox.filter((i) => i.id !== id);
    return delay(undefined);
  },
  async listTasks() {
    return delay(tasks);
  },
  async updateTaskState(id, state: TaskState) {
    const t = tasks.find((x) => x.id === id);
    if (t) {
      t.state = state;
      t.updated = new Date().toISOString();
    }
    return delay(undefined);
  },
};
