/**
 * Deterministic mock fixtures for the console.
 *
 * Every value is generated from a seeded PRNG so server and client renders are
 * byte-identical (no hydration drift). This stands in for the harness backend
 * until NEXT_PUBLIC_HARNESS_SOURCE=live points the client at the real graph.
 * The shapes match src/lib/harness/types.ts exactly so swapping the data source
 * changes nothing downstream.
 */
import {
  type Atom,
  type AtomKind,
  type MemoryCluster,
  type MemoryEdge,
  type EdgeKind,
  type Skill,
  type Room,
  type Run,
  type Head,
  type Provider,
  type HarnessKey,
  type UsagePeriod,
  type GithubConnection,
  type McpHubState,
  type AgentBinding,
  type InboxItem,
  type Task,
} from "./types";

// Seeded PRNG (mulberry32), mirroring the site's DotGrid determinism.
function mulberry32(seed: number) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const rnd = mulberry32(0x7e0a17);
const pick = <T,>(arr: T[]): T => arr[Math.floor(rnd() * arr.length)];
const isoAgo = (mins: number) => new Date(Date.now() - mins * 60_000).toISOString();

// --- Clusters (named, from the harness community detection) -----------------
export const CLUSTERS: MemoryCluster[] = [
  { id: "c-coord", label: "Coordination exhaust", count: 0, hue: 8 },
  { id: "c-substrate", label: "Substrate engine", count: 0, hue: 210 },
  { id: "c-graphql", label: "GraphQL MCP surface", count: 0, hue: 150 },
  { id: "c-embedded", label: "Embedded mode", count: 0, hue: 280 },
  { id: "c-deploy", label: "Deploy + Railway", count: 0, hue: 35 },
  { id: "c-design", label: "Design + console", count: 0, hue: 330 },
];

const KINDS: AtomKind[] = [
  "decision",
  "feedback",
  "solution",
  "postmortem",
  "preference",
  "note",
  "reflection",
  "handoff",
  "coordination",
  "source",
  "skill",
];

const TAG_POOL = [
  "rustyred", "graphql", "mcp", "embedded", "deploy", "railway", "harness",
  "coordination", "epistemic", "memory", "browser", "ios", "skills", "ensemble",
  "console", "tokens", "depth", "search",
];

const TITLES = [
  "GraphQL MCP surface collapses flat tools",
  "Embedded engine: link or drop the binary",
  "RedCoreGraphStore restart rehydration",
  "Coordination room tenant resolution 404",
  "Stream-based coordination read by cursor",
  "Storage spine eviction is O(k log n)",
  "EpistemicRAG shadow graph never mutates content",
  "Railway Dockerfile COPY drift recurs",
  "Default to drive, no scope-confirmation Qs",
  "Oxblood accent #8A2E29 holds 8.4:1 on white",
  "DotGrid ambient field is the depth win",
  "Dynamic Island unifies omnibar + TOC + search",
  "compute_code is search, not URL ingest",
  "Upsert REPLACES nodes, read-modify-write",
  "harness_run truncates at 16KB into a handle",
  "Skill lifecycle: draft to canonical",
  "Two-key model: inbound vs outbound",
  "Providers resolve credential_ref at run time",
  "Memory list must honor kind filter server side",
  "Velt cursors over a Yjs doc Velt does not own",
];

const SUMMARIES = [
  "A typed async-graphql schema wraps the flat THG tools, eight domains.",
  "Run RustyRed in-process over a local directory with no server.",
  "Writes go to the AOF; reads serve from a mirror rebuilt on open.",
  "Pass tenant_slug or the room returns Application not found.",
  "Append-only event streams replace turn-start room polling.",
  "The frontier reads the coldest tail without an O(n) scan.",
  "Shadow nodes carry epistemic standing; the content graph is untouched.",
  "A path-dep added without a COPY breaks the cloud build, not local.",
  "The answer is always yes; default to the maximal interpretation.",
  "Deep enough not to read orange, light enough not to read wine.",
  "A seeded dot field with mouse repulsion and a decaying ink trail.",
  "One bottom-center element carries five jobs via the island metaphor.",
];

function makeAtom(i: number): Atom {
  const cluster = pick(CLUSTERS);
  const kind: AtomKind = i < 130 ? "coordination" : pick(KINDS);
  const created = isoAgo(Math.floor(rnd() * 60 * 24 * 30));
  const updated = isoAgo(Math.floor(rnd() * 60 * 24 * 7));
  const angle = rnd() * Math.PI * 2;
  const radius = 0.18 + rnd() * 0.22;
  const cx = (CLUSTERS.indexOf(cluster) % 3) / 2;
  const cy = Math.floor(CLUSTERS.indexOf(cluster) / 3) / 1;
  const title =
    kind === "coordination"
      ? `coord: ${pick(["intent", "reflection", "presence", "mention", "record"])} ${pick(["codex", "claude-code", "claude-ai"])}`
      : pick(TITLES);
  return {
    id: `atom_${i.toString().padStart(3, "0")}`,
    title,
    kind,
    summary: pick(SUMMARIES),
    body: `# ${title}\n\n${pick(SUMMARIES)}\n\nThis atom is a projection of a graph node. Editing it edits the truth via \`self_revise\` / \`upsert_note\`, producing a revision rather than overwriting.\n\nSee [[${pick(TITLES)}]] for the related decision.`,
    tags: Array.from({ length: 1 + Math.floor(rnd() * 3) }, () => pick(TAG_POOL)),
    salience: Math.round(rnd() * 100) / 100,
    fitness: kind === "skill" ? Math.round(rnd() * 100) / 100 : undefined,
    source: kind === "source" ? pick(["pdf", "obsidian", "claude.ai", "codex"]) : undefined,
    created,
    updated,
    lifecycle: i % 17 === 0 ? "archived" : i % 29 === 0 ? "trash" : "active",
    links: rnd() > 0.6 ? [pick(TITLES)] : [],
    x: Math.min(0.98, Math.max(0.02, cx + Math.cos(angle) * radius)),
    y: Math.min(0.98, Math.max(0.02, cy + Math.sin(angle) * radius)),
    clusterId: cluster.id,
  };
}

export const ATOMS: Atom[] = Array.from({ length: 236 }, (_, i) => makeAtom(i));

// recompute cluster counts from generated atoms
for (const c of CLUSTERS) {
  c.count = ATOMS.filter((a) => a.clusterId === c.id && a.lifecycle === "active").length;
}

export const EDGES: MemoryEdge[] = (() => {
  const kinds: EdgeKind[] = ["supports", "contradicts", "refines", "derives", "cites", "wikilink"];
  const out: MemoryEdge[] = [];
  const active = ATOMS.filter((a) => a.lifecycle === "active");
  for (let i = 0; i < active.length; i++) {
    const edgeN = Math.floor(rnd() * 3);
    for (let e = 0; e < edgeN; e++) {
      const to = active[Math.floor(rnd() * active.length)];
      if (to.id === active[i].id) continue;
      out.push({
        id: `edge_${i}_${e}`,
        from: active[i].id,
        to: to.id,
        kind: pick(kinds),
      });
    }
  }
  return out;
})();

// --- Skills -----------------------------------------------------------------
const SKILL_MD = `---
name: rust-engineering
description: Apply Rust engineering discipline to harness crates, MCP servers, and PyO3 bridges.
---

# Rust Engineering

Use when writing, reviewing, or debugging Rust in the harness workspace.

## Checklist
- Build with the workspace toolchain; no per-crate drift.
- Prefer source-grounded edits over broad refactors.
- Run \`cargo test -p <crate>\` before claiming green.
`;

export const SKILLS: Skill[] = [
  {
    id: "skill_rust",
    name: "rust-engineering",
    description: "Rust discipline for harness crates, MCP servers, PyO3 bridges.",
    status: "canonical",
    contentHash: "sha256:b2c1...9af3",
    version: "v4",
    updated: isoAgo(60 * 5),
    files: [
      { path: "SKILL.md", language: "markdown", content: SKILL_MD },
      { path: "checklists/review.md", language: "markdown", content: "# Review\n- types\n- tests\n- clippy\n" },
    ],
  },
  {
    id: "skill_design",
    name: "design-engineering",
    description: "Token-and-axe render-and-check gate for CSS, type, motion, a11y receipts.",
    status: "validated",
    contentHash: "sha256:7d10...c4e2",
    version: "v2",
    updated: isoAgo(60 * 26),
    files: [{ path: "SKILL.md", language: "markdown", content: "---\nname: design-engineering\ndescription: render, lint tokens, run axe, pass or fail.\n---\n# Design Engineering\n" }],
  },
  {
    id: "skill_compute_code",
    name: "compute_code",
    description: "Route code search to the right RustyRed inline graph algorithm.",
    status: "advisory",
    contentHash: "sha256:1188...aa90",
    version: "v1",
    updated: isoAgo(60 * 50),
    files: [{ path: "SKILL.md", language: "markdown", content: "---\nname: compute_code\ndescription: graph-structural code ranking.\n---\n# compute_code\n" }],
  },
  {
    id: "skill_obsidian",
    name: "obsidian-sync",
    description: "Mirror memory docs into a vault; write note edits back into the graph.",
    status: "draft",
    contentHash: "sha256:0000...0001",
    version: "v0",
    updated: isoAgo(60 * 2),
    files: [{ path: "SKILL.md", language: "markdown", content: "---\nname: obsidian-sync\ndescription: draft.\n---\n# Obsidian Sync\n" }],
  },
];

// --- Rooms ------------------------------------------------------------------
export const ROOMS: Room[] = [
  {
    id: "room_console",
    name: "harness-console",
    topic: "Build the Theorems Harness web console",
    updated: isoAgo(3),
    participants: [
      { actor: "claude-code", presence: "live", lastSeen: isoAgo(0) },
      { actor: "codex", presence: "idle", lastSeen: isoAgo(14) },
      { actor: "claude-ai", presence: "away", lastSeen: isoAgo(120) },
    ],
    events: [
      { id: "rev1", kind: "intent", actor: "claude-code", text: "Building apps/harness-console foundation: tokens, shell, lib client.", at: isoAgo(8) },
      { id: "rev2", kind: "decision", actor: "claude-code", text: "Tailwind v3 + hand-authored shadcn primitives for deterministic builds.", at: isoAgo(6) },
      { id: "rev3", kind: "reflection", actor: "codex", text: "Left the console lane to CC; staying on core crates.", at: isoAgo(14) },
      { id: "rev4", kind: "message", actor: "claude-ai", text: "Palette locked: oxblood #8A2E29, status green #5f7d4f.", at: isoAgo(120), mentions: ["claude-code"] },
    ],
  },
  {
    id: "room_embedded",
    name: "rustyred-embedded",
    topic: "North Star E0 embedded mode",
    updated: isoAgo(90),
    participants: [
      { actor: "codex", presence: "idle", lastSeen: isoAgo(90) },
      { actor: "claude-code", presence: "away", lastSeen: isoAgo(300) },
    ],
    events: [
      { id: "rev5", kind: "record", actor: "claude-code", text: "E0.5 folder tree done, 11 tests green, clippy-clean.", at: isoAgo(300) },
      { id: "rev6", kind: "tension", actor: "codex", text: "DocTree-as-ColdIndex (B3) is a separate store concern, not blocking E0.5.", at: isoAgo(120) },
    ],
  },
];

// --- Runs -------------------------------------------------------------------
export const RUNS: Run[] = [
  {
    id: "run_7f3a",
    goal: "Collapse flat MCP tools into a typed GraphQL surface (A7 cutover)",
    status: "complete",
    verdict: "aligned",
    stepCount: 12,
    started: isoAgo(180),
    finished: isoAgo(120),
    steps: Array.from({ length: 12 }, (_, i) => ({
      index: i,
      at: isoAgo(180 - i * 5),
      kind: pick(["transition", "tool_call", "observation", "head_contribution"]),
      actor: pick(["claude", "deepseek"]),
      summary: pick([
        "recall prior cutover decision",
        "graphql_introspect for SDL coverage",
        "retain-filter hides covered flat tools",
        "graphql_mutate rememberMemory through the typed surface",
        "verdict: aligned, 90 mcp tests green",
      ]),
      detail: "step detail payload",
    })),
  },
  {
    id: "run_9c12",
    goal: "Draft the harness console surface from two specs",
    status: "running",
    verdict: "pending",
    stepCount: 4,
    started: isoAgo(9),
    steps: Array.from({ length: 4 }, (_, i) => ({
      index: i,
      at: isoAgo(9 - i * 2),
      kind: pick(["transition", "tool_call", "observation"]),
      actor: "claude",
      summary: pick(["observe repo state", "author token file", "scaffold lib client", "fan out surfaces"]),
    })),
  },
  {
    id: "run_2b88",
    goal: "Wire theorem-gateway askAgent to GL-Fusion",
    status: "error",
    verdict: "flagged",
    stepCount: 7,
    started: isoAgo(1440),
    finished: isoAgo(1430),
    steps: Array.from({ length: 7 }, (_, i) => ({
      index: i,
      at: isoAgo(1440 - i * 3),
      kind: pick(["tool_call", "observation"]),
      actor: "claude",
      summary: pick(["assemble code-KG context", "POST GL-Fusion", "GLFUSION_URL not configured", "return honest empty answer"]),
    })),
  },
];

// --- Agent binding (composed Theorem agent) ---------------------------------
export const HEADS: Head[] = [
  { id: "claude", model: "claude-opus-4-8", provider: "anthropic", role: "proposer / peer", keyStatus: "ok" },
  { id: "deepseek", model: "deepseek-reasoner", provider: "deepseek", role: "peer / verifier", keyStatus: "ok" },
  { id: "mistral", model: "mistral-large", provider: "mistral", role: "peer", keyStatus: "missing" },
];

export const BINDING: AgentBinding = {
  bindingId: "agent:theorem",
  heads: HEADS,
  scope: {
    memoryScopes: ["rustyredcore-theorem-production", "console"],
    rooms: ["harness-console", "rustyred-embedded"],
  },
};

// --- Keys (inbound) ---------------------------------------------------------
export const KEYS: HarnessKey[] = [
  { id: "key_1", name: "Claude Code (laptop)", prefix: "hk_live_8f3a2b", created: isoAgo(60 * 24 * 12), lastUsed: isoAgo(4), scopes: ["memory:read", "memory:write", "coordination:read", "run:write"] },
  { id: "key_2", name: "Codex (workstation)", prefix: "hk_live_2c19de", created: isoAgo(60 * 24 * 9), lastUsed: isoAgo(40), scopes: ["memory:read", "memory:write", "coordination:read"] },
  { id: "key_3", name: "CI smoke", prefix: "hk_live_55a0f1", created: isoAgo(60 * 24 * 3), scopes: ["run:read"] },
];

// --- Providers (outbound) ---------------------------------------------------
export const PROVIDERS: Provider[] = [
  { name: "anthropic", label: "Anthropic", keyStatus: "ok", defaultModel: "claude-opus-4-8", mode: "byok", credentialRef: "cred://anthropic/default" },
  { name: "deepseek", label: "DeepSeek", keyStatus: "ok", defaultModel: "deepseek-reasoner", mode: "byok", credentialRef: "cred://deepseek/default" },
  { name: "mistral", label: "Mistral", keyStatus: "missing", defaultModel: "mistral-large", mode: "credits" },
  { name: "openai", label: "OpenAI", keyStatus: "invalid", defaultModel: "gpt-4o", mode: "byok", credentialRef: "cred://openai/default" },
];

// --- Usage ------------------------------------------------------------------
export const USAGE: UsagePeriod = {
  requests: 4128,
  toolCalls: 18743,
  limit: 10000,
  plan: "Free",
  payPerUse: false,
  periodLabel: "This period (Jun 1 - Jun 20)",
  series: Array.from({ length: 20 }, (_, i) => ({
    label: `Jun ${i + 1}`,
    value: Math.floor(60 + rnd() * 380),
  })),
};

// --- Connections + MCP Hub --------------------------------------------------
export const GITHUB: GithubConnection = {
  connected: true,
  account: "Travis-Gilbert",
  repos: [
    { id: "r1", owner: "Travis-Gilbert", name: "Theorem", status: "indexed", symbols: 48213, updated: isoAgo(120) },
    { id: "r2", owner: "Travis-Gilbert", name: "Theseus", status: "ingesting", symbols: 12044, updated: isoAgo(2) },
    { id: "r3", owner: "Travis-Gilbert", name: "Open-Flint-Atlas", status: "queued", updated: isoAgo(0) },
    { id: "r4", owner: "Travis-Gilbert", name: "Reflexive-red", status: "error", updated: isoAgo(400) },
  ],
};

export const MCP_HUB: McpHubState = {
  namespaces: [
    { id: "ns_memory", label: "memory", description: "recall, remember, relate, self_revise, forget.", verbs: 11, enabled: true },
    { id: "ns_graph", label: "graph", description: "nodes, neighbors, vector, fulltext, spatial, symbolic.", verbs: 16, enabled: true },
    { id: "ns_coord", label: "coordination", description: "rooms, intents, streams, presence, mentions.", verbs: 14, enabled: true },
    { id: "ns_code", label: "code", description: "compute_code, code_ingest, code graph search.", verbs: 8, enabled: true },
    { id: "ns_epistemic", label: "epistemic", description: "shadow graph neighbors, frontier, enrich.", verbs: 5, enabled: false },
    { id: "ns_harness", label: "harness", description: "run lifecycle, replay, fork, compare, jobs.", verbs: 12, enabled: true },
  ],
  brokered: [
    { id: "b1", name: "firecrawl", transport: "http", url: "https://mcp.firecrawl.dev", status: "connected", tools: 9 },
    { id: "b2", name: "railway", transport: "http", url: "https://mcp.railway.app", status: "connected", tools: 41 },
    { id: "b3", name: "context7", transport: "http", url: "https://mcp.context7.com", status: "disabled", tools: 2 },
  ],
};

// --- Inbox (mentions / runs / system) ---------------------------------------
export const INBOX: InboxItem[] = [
  {
    id: "inb_1",
    kind: "mention",
    title: "codex mentioned you in harness-console",
    from: "codex",
    preview: "@claude-code the token bridge looks right; I'll take the inbox board UX.",
    body: "@claude-code the token bridge looks right; I'll take the inbox board UX. Heads up that focalboard is a full app, not a component, so I'm building the board with dnd-kit. Picking up after your canvas lands.",
    at: isoAgo(6),
    read: false,
    room: "harness-console",
    href: "/rooms",
  },
  {
    id: "inb_2",
    kind: "run",
    title: "Run complete: A7 cutover",
    from: "harness",
    preview: "Alignment verdict: aligned. 90 mcp tests green.",
    body: "composed_agent_run finished the GraphQL cutover. Alignment gate: aligned. 12 steps, 90 mcp tests green. Open the ledger to replay.",
    at: isoAgo(34),
    read: false,
    href: "/runs",
  },
  {
    id: "inb_3",
    kind: "job",
    title: "Job picked up: ingest Theseus repo",
    from: "theorem-receiver",
    preview: "claude head claimed job_8f3a and started a session.",
    body: "The receiver started job_8f3a (ingest Theseus into the code graph) on the claude head with start_session_ref recorded. It will append a job_note on exit.",
    at: isoAgo(58),
    read: true,
    href: "/inbox",
  },
  {
    id: "inb_4",
    kind: "system",
    title: "Provider key invalid: OpenAI",
    from: "system",
    preview: "The OpenAI key failed validation; heads on that provider will be skipped.",
    body: "Validating the OpenAI provider key returned 401. The composed agent will skip heads bound to OpenAI until a valid key is added on the Providers surface.",
    at: isoAgo(180),
    read: true,
    href: "/providers",
  },
  {
    id: "inb_5",
    kind: "mention",
    title: "claude-ai left a reflection",
    from: "claude-ai",
    preview: "Palette locked: oxblood #8A2E29, status green #5f7d4f.",
    body: "Reflection for the next head: palette is locked (oxblood #8A2E29, status green #5f7d4f). The Dynamic Island carries the reuno-ui ai-input treatment. Don't reintroduce a second accent.",
    at: isoAgo(300),
    read: true,
    room: "harness-console",
    href: "/rooms",
  },
];

// --- Tasks (Dispatch v2 jobs) -----------------------------------------------
export const TASKS: Task[] = [
  { id: "job_8f3a", title: "Ingest Theseus repo into the code graph", state: "running", priority: "high", targetHead: "claude", updated: isoAgo(58), note: "started_session_ref recorded", runId: "run_7f3a" },
  { id: "job_2c19", title: "Retokenize the file-tree component to the palette", state: "queued", priority: "normal", targetHead: "codex", updated: isoAgo(20) },
  { id: "job_55a0", title: "Wire askAgent to GL-Fusion endpoint", state: "blocked", priority: "high", targetHead: "claude", updated: isoAgo(1440), note: "GLFUSION_URL not configured" },
  { id: "job_77b1", title: "Draft the mail/inbox three-pane layout", state: "done", priority: "normal", targetHead: "claude", updated: isoAgo(90) },
  { id: "job_9d22", title: "Add per-file vector embedding to fs_write", state: "done", priority: "low", targetHead: "codex", updated: isoAgo(2880) },
  { id: "job_0e44", title: "Deploy commonplace-api to Railway", state: "queued", priority: "normal", updated: isoAgo(5) },
];
