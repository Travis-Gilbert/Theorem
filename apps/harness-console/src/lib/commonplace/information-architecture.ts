export type CommonplacePlacement = "page" | "omnibar-capability" | "data-view" | "quick-action" | "system";

export interface CommonplaceIaItem {
  id: string;
  label: string;
  placement: CommonplacePlacement;
  description: string;
  href?: string;
  count?: string;
}

export interface CommonplaceToolboxGroup {
  id: "see" | "add";
  label: string;
  items: readonly CommonplaceIaItem[];
}

export interface CommonplaceDataViewDescriptor {
  id: string;
  label: string;
  viewDescriptorId: string;
  status?: "enabled" | "deferred";
  deferredReason?: string;
  objectTypes: readonly string[];
  renderers: readonly string[];
  actions: readonly string[];
  query: {
    types: readonly string[];
    live: boolean;
    rank?: readonly string[];
    slice?: readonly string[];
  };
}

export interface CommonplaceSceneRenderer {
  id: string;
  label: string;
  capability: string;
  status: "registered" | "planned";
}

export const COMMONPLACE_IA_RULES: readonly { placement: CommonplacePlacement; test: string }[] = [
  { placement: "page", test: "Do I dwell here to work?" },
  { placement: "omnibar-capability", test: "Is this a setting on the agent, not a room?" },
  { placement: "data-view", test: "Is this a lens over my stuff?" },
  { placement: "quick-action", test: "Do I trigger it and leave?" },
  { placement: "system", test: "Is this setup, not work?" },
] as const;

export const COMMONPLACE_WORK_PAGES: readonly CommonplaceIaItem[] = [
  {
    id: "index",
    label: "Index",
    placement: "page",
    description: "Daily triage, confidence-line decisions, and automatic filing.",
    href: "/commonplace/index",
  },
  {
    id: "threads",
    label: "Threads",
    placement: "page",
    description: "Persistent agent conversations tied to produced work.",
    href: "/commonplace/threads",
    count: "3",
  },
  {
    id: "write",
    label: "Write",
    placement: "page",
    description: "Notebooks, notes, and compose as one writing destination.",
    href: "/commonplace/write",
  },
  {
    id: "code",
    label: "Code",
    placement: "page",
    description: "Coding harness with files, editor, diffs, agent run trace, and terminal.",
    href: "/Commonplace",
    count: "4",
  },
  {
    id: "artifacts",
    label: "Artifacts",
    placement: "page",
    description: "Generated outputs and full-canvas interactive scenes.",
    href: "/commonplace/artifacts",
    count: "1",
  },
] as const;

export const COMMONPLACE_OMNIBAR_CAPABILITIES: readonly CommonplaceIaItem[] = [
  {
    id: "instant-kg",
    label: "Instant KG",
    placement: "omnibar-capability",
    description: "Build a knowledge graph from current material on the fly.",
  },
  {
    id: "web",
    label: "Web",
    placement: "omnibar-capability",
    description: "Allow the agent to browse and search.",
  },
  {
    id: "attach",
    label: "Attach",
    placement: "omnibar-capability",
    description: "Bring files or context into the current turn.",
  },
  {
    id: "tier",
    label: "Tier",
    placement: "omnibar-capability",
    description: "Gate reasoning head count: simple, difficult, or max.",
  },
  {
    id: "git-aware",
    label: "Git-aware",
    placement: "omnibar-capability",
    description: "Let the agent read and act against RustyRed git context.",
  },
  {
    id: "deepen",
    label: "Deepen",
    placement: "omnibar-capability",
    description: "Run heavier background passes after a save or answer.",
  },
] as const;

export const COMMONPLACE_DATA_VIEWS: readonly CommonplaceIaItem[] = [
  {
    id: "files",
    label: "Files",
    placement: "data-view",
    description: "Uploaded file objects and RustyRed file front end.",
    count: "4",
  },
  {
    id: "graph",
    label: "Graph",
    placement: "data-view",
    description: "Graph viewer with snapshot, diff, branch, and restore history affordance.",
  },
  {
    id: "table",
    label: "Table",
    placement: "data-view",
    description: "Structured records and no-code database display.",
  },
  {
    id: "timeline",
    label: "Timeline",
    placement: "data-view",
    description: "Temporal view of objects.",
    count: "3",
  },
  {
    id: "clips",
    label: "Clips",
    placement: "data-view",
    description: "Clipped web and media content.",
  },
] as const;

export const COMMONPLACE_DATA_VIEW_DESCRIPTORS: readonly CommonplaceDataViewDescriptor[] = [
  {
    id: "files",
    label: "Files",
    viewDescriptorId: "file-tree",
    objectTypes: ["file", "folder"],
    renderers: ["react-arborist", "@uiw/react-codemirror"],
    actions: ["open", "attach", "route", "inspect provenance"],
    query: { types: ["file"], live: true, rank: ["recent", "graph"], slice: ["space"] },
  },
  {
    id: "graph",
    label: "Graph",
    viewDescriptorId: "graph",
    objectTypes: ["node", "edge", "cluster"],
    renderers: ["@cosmos.gl/graph", "React Flow for close workflow graphs"],
    actions: ["snapshot", "diff", "branch", "restore"],
    query: { types: ["graph_node", "graph_edge"], live: true, rank: ["ppr", "bm25"] },
  },
  {
    id: "table",
    label: "Table",
    viewDescriptorId: "table",
    objectTypes: ["record", "task", "project", "schema"],
    renderers: ["@tanstack/react-table", "@nocobase/client isolated app bridge", "shadcn form primitives"],
    actions: ["sort", "filter", "inline edit", "promote schema"],
    query: { types: ["record", "task", "project"], live: true, rank: ["field"] },
  },
  {
    id: "map",
    label: "Map",
    viewDescriptorId: "map",
    status: "deferred",
    deferredReason: "Map surface deferred; Deck.gl contract is installed for future coordinate layers and MapLibre is intentionally skipped.",
    objectTypes: ["place", "event", "asset"],
    renderers: ["@deck.gl/react", "@deck.gl/layers"],
    actions: ["cluster", "route", "open related graph"],
    query: { types: ["place", "event"], live: true, slice: ["space"], rank: ["geo"] },
  },
  {
    id: "timeline",
    label: "Timeline",
    viewDescriptorId: "timeline",
    objectTypes: ["event", "task", "thread", "artifact"],
    renderers: ["shadcn timeline primitives"],
    actions: ["jump", "filter", "open related object"],
    query: { types: ["event", "task", "thread", "artifact"], live: true, slice: ["valid_time"], rank: ["time"] },
  },
  {
    id: "clips",
    label: "Clips",
    viewDescriptorId: "clips",
    objectTypes: ["clip", "web_capture", "media"],
    renderers: ["shadcn list primitives", "OpenUI evidence cards"],
    actions: ["open source", "attach", "summarize"],
    query: { types: ["clip", "web_capture"], live: true, rank: ["bm25", "recent"] },
  },
] as const;

export const COMMONPLACE_SCENE_RENDERERS: readonly CommonplaceSceneRenderer[] = [
  {
    id: "scene-artifact-preview",
    label: "SceneArtifactPreview",
    capability: "Mount a saved ScenePackage or generated interface preview.",
    status: "registered",
  },
  {
    id: "scene-package-card",
    label: "ScenePackageCard",
    capability: "Show manifest, provenance, datasets, traces, and fallback status.",
    status: "planned",
  },
  {
    id: "scene-control-strip",
    label: "SceneControlStrip",
    capability: "Replay, inspect, fork, and confirm generated scene patches.",
    status: "planned",
  },
] as const;

export const COMMONPLACE_TOOLBOX: readonly CommonplaceToolboxGroup[] = [
  {
    id: "see",
    label: "See",
    items: [
      {
        id: "terminal",
        label: "Terminal",
        placement: "quick-action",
        description: "Open a shell and return to the current workspace.",
      },
      {
        id: "cluster",
        label: "Cluster",
        placement: "quick-action",
        description: "Jump to a related cluster.",
      },
      {
        id: "timeline",
        label: "Timeline",
        placement: "quick-action",
        description: "Open the current object's temporal lens.",
      },
    ],
  },
  {
    id: "add",
    label: "Add",
    items: [
      {
        id: "note",
        label: "Note",
        placement: "quick-action",
        description: "Create a note from the current context.",
      },
      {
        id: "task",
        label: "Task",
        placement: "quick-action",
        description: "Create a task and return.",
      },
      {
        id: "reminder",
        label: "Reminder",
        placement: "quick-action",
        description: "Create a reminder.",
      },
      {
        id: "project",
        label: "Project",
        placement: "quick-action",
        description: "Create a project container.",
      },
    ],
  },
] as const;

export const COMMONPLACE_ACCOUNT_ITEMS: readonly CommonplaceIaItem[] = [
  {
    id: "account",
    label: "Account",
    placement: "system",
    description: "Profile, billing, and account settings.",
    href: "/commonplace/account",
  },
  {
    id: "agents",
    label: "Agents",
    placement: "system",
    description: "Agent configuration, heads, and bring-your-own-agent over ACP.",
    href: "/commonplace/agents",
    count: "4",
  },
  {
    id: "engine",
    label: "Engine",
    placement: "system",
    description: "Substrate status and configuration.",
    href: "/commonplace/engine",
  },
  {
    id: "desktop",
    label: "Desktop",
    placement: "system",
    description: "Desktop app and connectors.",
    href: "/commonplace/desktop",
    count: "3",
  },
  {
    id: "settings",
    label: "Settings",
    placement: "system",
    description: "App preferences.",
    href: "/commonplace/settings",
  },
] as const;

export const RETIRED_COMMONPLACE_PAGES = ["Models"] as const;
