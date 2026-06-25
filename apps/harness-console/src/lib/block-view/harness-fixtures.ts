import type {
  BlockHost,
  ObjectAction,
  ObjectActionReceipt,
  ObjectRef,
  ObjectSet,
  ObjectShape,
  Result,
  ThemeTokens,
  ViewDescriptor,
} from "./types";
import { viewsForShape } from "./registry";

const noop = () => undefined;

export const COMMONPLACE_THEME_TOKENS: ThemeTokens = {
  color: {
    bg: "var(--bg)",
    surface: "var(--surface)",
    surface_2: "var(--surface-2)",
    ink: "var(--ink)",
    muted: "var(--muted)",
    accent: "var(--ox)",
    accent_tint: "var(--ox-tint)",
    live: "var(--live)",
    warn: "var(--warn)",
    line: "var(--line)",
  },
  space: {
    one: "var(--space-1)",
    two: "var(--space-2)",
    three: "var(--space-3)",
    four: "var(--space-4)",
    five: "var(--space-5)",
    six: "var(--space-6)",
  },
  typography: {
    label: "var(--text-label)",
    body: "var(--text-body)",
    subhead: "var(--text-subhead)",
    title: "var(--text-title)",
    mono: "var(--font-mono)",
    sans: "var(--font-sans)",
    serif: "var(--font-serif)",
  },
  radius: {
    sm: "var(--radius-sm)",
    md: "var(--radius-md)",
    lg: "var(--radius-lg)",
  },
};

export const HARNESS_OBJECT_SETS = {
  files: makeSet([
    file("file:root", "workspace", "", "false", ["file:src", "file:tests"]),
    file("file:src", "src", "", "false", ["file:src-agent", "file:src-contract"]),
    file(
      "file:src-agent",
      "src/agent.ts",
      "export function runAgent(target) {\n  return host.emit({ kind: \"run_agent\", target, tier: \"difficult\" });\n}\n",
      "true",
    ),
    file(
      "file:src-contract",
      "src/block-contract.ts",
      "export type BlockHost = {\n  query(q: ObjectQuery): ObjectSet;\n  emit(a: ObjectAction): Promise<Result>;\n};\n",
    ),
    file("file:tests", "tests", "", "false", ["file:tests-contract"]),
    file("file:tests-contract", "tests/block-contract.test.ts", "expect(viewsFor(shape)).toContain(\"patch-review\");\n"),
  ]),
  patch: makeSet([
    {
      id: "patch:block-contract",
      type: "patch",
      properties: {
        title: "Route coding harness panels through block descriptors",
        before: "const panel = renderPatchReview(run.patch);\nconst tree = renderFileTree(files);\n",
        after:
          "const patchView = viewsFor(patchSet.shape).find((view) => view.id === \"patch-review\");\nconst treeView = viewsFor(fileSet.shape).find((view) => view.id === \"file-tree\");\n",
        hunks: ["src/workspace.tsx", "src/block-view/registry.tsx"],
        status: "review",
      },
      relations: { ABOUT: ["run:block-contract"] },
      axes: { valid: { from_ms: 1782420000000 }, embeddable: true },
    },
  ]),
  thread: makeSet([
    message("message:1", "user", "Make the coding harness panels blocks over objects, not bespoke panes."),
    message("message:2", "assistant", "Mapped file, patch, run, tool, context, and terminal panels to ObjectSet shapes."),
  ]),
  trace: makeSet([
    step("step:observe", "observe", "done", "Loaded object/block contract and token system."),
    step("step:design", "design_engineering", "done", "Confirmed tokenized panel chrome and reduced-motion path."),
    step("step:validate", "validate", "running", "Running typecheck, lint, and design-math/token checks."),
  ]),
  tools: makeSet([
    tool("tool:diff", "patch_review", "ok"),
    tool("tool:terminal", "terminal_session", "ok"),
    tool("tool:sandbox", "sandpack_preview", "queued"),
  ]),
  context: makeSet([
    artifact("context:block-contract", "Block/view contract", "Stable four-method host plus ObjectQuery/ObjectAction/ViewDescriptor shapes."),
    artifact("context:ui-ux", "UI/UX North Star", "NocoBase for structured records, native RustyRed blocks for the harness and graph moat."),
  ]),
  terminal: makeSet([
    {
      id: "terminal:validation",
      type: "terminal_session",
      properties: {
        command: "npm run lint",
        output: "eslint .\nDesign math lint passed: spacing and token checks clean.\n",
        status: "done",
      },
      relations: {},
      axes: {},
    },
  ]),
  runs: makeSet([
    run("run:queued", "Schema form generator", "queued", "Waiting on tool schema hydration."),
    run("run:active", "Block contract shell", "running", "Rendering object views through descriptors."),
    run("run:blocked", "Live record changefeed", "blocked", "Poll fallback is active until record-level stream lands."),
    run("run:done", "CommonPlace contract", "done", "Rust and TS contract surfaces validated."),
  ]),
} as const;

export const mockBlockHost: BlockHost = {
  tokens: COMMONPLACE_THEME_TOKENS,
  query: () => HARNESS_OBJECT_SETS.files,
  async emit(action: ObjectAction): Promise<Result<ObjectActionReceipt>> {
    return {
      ok: true,
      value: {
        action_kind: action.kind,
        status: action.kind === "open" || action.kind === "select" ? "accepted" : "applied",
        target_ids: action.kind === "open" ? [action.id] : [],
      },
    };
  },
  viewsFor(shape: ObjectShape): readonly ViewDescriptor[] {
    return viewsForShape(shape);
  },
};

function makeSet(objects: readonly ObjectRef[]): ObjectSet {
  return {
    objects,
    shape: inferShape(objects),
    subscribe: () => noop,
  };
}

function inferShape(objects: readonly ObjectRef[]): ObjectShape {
  const types = new Set<string>();
  const fields = new Set<string>();
  const relations = new Map<string, { edge: string; dir: "out" }>();
  const axes = { spatial: false, temporal: false, embeddable: false };

  for (const object of objects) {
    types.add(object.type);
    Object.keys(object.properties).forEach((field) => fields.add(field));
    Object.keys(object.relations ?? {}).forEach((edge) => relations.set(edge, { edge, dir: "out" }));
    axes.spatial ||= Boolean(object.axes?.h3);
    axes.temporal ||= Boolean(object.axes?.valid);
    axes.embeddable ||= Boolean(object.axes?.embeddable);
  }

  return {
    types: [...types].sort(),
    fields: [...fields].sort(),
    relations: [...relations.values()].sort((a, b) => a.edge.localeCompare(b.edge)),
    axes,
    cardinality: objects.length === 0 ? "empty" : objects.length === 1 ? "one" : "many",
  };
}

function file(id: string, path: string, content: string, active = "false", contains: readonly string[] = []): ObjectRef {
  return {
    id,
    type: "file",
    properties: { path, content, active, title: path.split("/").pop() ?? path },
    relations: contains.length ? { CONTAINS: contains } : {},
    axes: { embeddable: Boolean(content) },
  };
}

function message(id: string, role: string, content: string): ObjectRef {
  return {
    id,
    type: "agent_message",
    properties: { role, content, title: role },
    relations: {},
    axes: { valid: { from_ms: 1782420000000 } },
  };
}

function step(id: string, kind: string, status: string, summary: string): ObjectRef {
  return {
    id,
    type: "run_step",
    properties: { kind, status, summary, title: kind },
    relations: {},
    axes: { valid: { from_ms: 1782420000000 } },
  };
}

function tool(id: string, name: string, status: string): ObjectRef {
  return {
    id,
    type: "tool_activity",
    properties: { name, status, title: name },
    relations: {},
    axes: {},
  };
}

function artifact(id: string, title: string, summary: string): ObjectRef {
  return {
    id,
    type: "context_atom",
    properties: { title, summary },
    relations: {},
    axes: { embeddable: true },
  };
}

function run(id: string, title: string, status: string, summary: string): ObjectRef {
  return {
    id,
    type: "agent_run",
    properties: { title, status, summary },
    relations: { ABOUT: ["patch:block-contract"] },
    axes: { valid: { from_ms: 1782420000000 }, embeddable: true },
  };
}
