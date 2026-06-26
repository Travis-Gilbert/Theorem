# UI Component Sourcing Migration Plan

Date: 2026-06-26
Branch: `Travis-Gilbert/commonplace-workspace-blocks`
Related PR: https://github.com/Travis-Gilbert/Theorem/pull/43
Primary spec: `docs/plans/commonplace/SPEC-UI-COMPONENT-SOURCING-AND-RESKIN.md`

## Decision

The current `/Commonplace` PR should keep the block/view contract and the native CommonPlace page-shell direction, but the render layer needs a second pass. Generic interface patterns must move from hand-built panels to sourced component adapters. Bespoke code remains valid only for Theorem/CommonPlace-specific meaning: provenance, context selection, run lifecycle, routing confidence, and the shell charm that makes the page feel like CommonPlace.

The migration should not try to "token-sync" a standalone harness. It should build a CommonPlace page surface whose blocks mount sourced components through `ViewDescriptor` adapters, with CommonPlace tokens bridging each upstream library.

## Research Corrections

These source checks change the implementation plan:

- Sandpack is useful for browser code execution previews, but its repository currently ships under Apache-2.0, not MIT. Source: https://github.com/codesandbox/sandpack and https://raw.githubusercontent.com/codesandbox/sandpack/main/LICENSE.
- NocoBase is not safe to treat as a plain Apache-2.0 component source without legal review. The repository contains `LICENSE-APACHE.txt`, but the root `package.json` declares `AGPL-3.0`, and `LICENSE.txt` adds NocoBase-specific restrictions including restrictions on public no-code, low-code, AI platform SaaS/PaaS use. Use NocoBase as a reference or separately deployed/isolated surface only after license review. Sources: https://github.com/nocobase/nocobase, https://raw.githubusercontent.com/nocobase/nocobase/main/package.json, and https://raw.githubusercontent.com/nocobase/nocobase/main/LICENSE.txt.
- React Bits is not plain MIT; the repository reports "MIT + Commons Clause". Use it only for copied decorative/artifact components after checking the exact component license path. Source: https://github.com/DavidHDev/react-bits.
- ERPNext remains reference-only because it is GPL-3.0. Source: https://github.com/frappe/erpnext.
- Tiptap is MIT, but BlockNote is not MIT in the current package metadata. `@blocknote/core` and `@blocknote/react` declare MPL-2.0, and the BlockNote README says XL packages are GPL-3.0/commercial. Use BlockNote carefully: prefer ordinary core/react packages, avoid XL packages unless the license path is approved, and publish local BlockNote source modifications if MPL-2.0 requires it. Sources: https://github.com/TypeCellOS/BlockNote, https://raw.githubusercontent.com/TypeCellOS/BlockNote/main/packages/core/package.json, and https://raw.githubusercontent.com/TypeCellOS/BlockNote/main/README.md.
- CodeMirror remains the safest web editor/diff spine. Its docs expose `EditorView.theme` for scoped themes and `@codemirror/merge` for side-by-side merge views. Source: https://codemirror.net/docs/ref/.
- `@uiw/react-codemirror` should be the React adapter for ordinary editor panels. The repo already depends on it and uses it in `MarkdownEditor` and `CollaborativeEditor`, so the migration should generalize those patterns instead of mounting raw `EditorView` by hand. The same uiw repo publishes `react-codemirror-merge`, which should be evaluated as the React adapter around `@codemirror/merge`. Source: https://github.com/uiwjs/react-codemirror.git.
- assistant-ui is the right chat spine: `AssistantRuntimeProvider`, `Thread`, `Message`, `Composer`, `ActionBar`, tool-call rendering, approvals, and integrations for AG-UI, A2A, OpenCode, LangGraph/LangChain, AI SDK, and custom runtimes. Source: https://github.com/assistant-ui/assistant-ui.
- OpenUI is the right generative surface because its library, prompt generator, parser, and renderer constrain model output to registered components. Source: https://github.com/thesysdev/openui.

## Current PR Gap

The branch currently proves the block/view contract and has a CommonPlace-looking route, but these panels are still hand-built:

| Current surface | Current issue | Target upstream owner |
| --- | --- | --- |
| `CodeWorkspaceShell` | custom split layout and shell CSS | `react-resizable-panels` plus shadcn primitives |
| `FileTreePanel` | hand-built rows/buttons | `react-arborist` |
| `CodeEditorPanel` | mostly static code presentation | CodeMirror 6 through `@uiw/react-codemirror`, reusing existing local CodeMirror/Yjs utilities |
| `PatchReviewPanel` | hand-built diff rows | `react-codemirror-merge` or raw `@codemirror/merge` for web, Hunk only for terminal/OpenTUI review |
| `AgentThreadPanel` | hand-built chat cards | assistant-ui `Thread`, `Message`, `Composer`, `ActionBar` |
| `ToolActivityPanel` | bespoke operation log | assistant-ui tool-call rendering with a thin Theorem event adapter |
| `TerminalPanel` | fake terminal text block | `xterm.js` with fit and webgl addons |
| `AgentRunBoard` | allowed bespoke, but too raw | shadcn Card/Badge plus dnd-kit |
| `RunTraceTimeline` | allowed bespoke, but too raw | shadcn primitives, optional react-chrono only if it fits |
| `ContextArtifactDrawer` | allowed bespoke | shadcn Sheet/Table over Theseus context data |

## Adapter Contract

Extend the `ViewDescriptor` metadata before installing every library. This makes sourcing testable instead of a verbal rule.

```ts
type ViewSource = {
  package: string;
  component: string;
  mode: "vendor" | "reskin" | "wrap" | "fork" | "bespoke";
  regime: "css-vars" | "ant-tokens" | "scene";
  allowedBespokeReason?: string;
};

type ViewDescriptor = {
  id: string;
  accepts: ObjectShapeMatch[];
  priority?: number;
  source: ViewSource;
  render: (props: ViewRenderProps) => React.ReactNode;
};
```

Add a small descriptor audit that fails when:

- `mode !== "bespoke"` and `source.package` is missing.
- `mode === "bespoke"` and `allowedBespokeReason` is missing.
- a non-bespoke descriptor renders through the generic hand-built block frame instead of a source adapter.

## Token Bridges

Keep one CommonPlace token set as the source of truth. Add thin bridges by library instead of styling every panel directly:

| Bridge | Consumer |
| --- | --- |
| `commonplace.css` custom properties | shell, shadcn, assistant-ui, React Flow, OpenUI registered components |
| `commonplaceCodeMirrorTheme` | CodeMirror editor and `@codemirror/merge` |
| `commonplaceXtermTheme` | `xterm.js` terminal |
| `commonplaceReactFlowTheme` | workflow graph nodes, edges, controls |
| `commonplaceAssistantComponents` | assistant-ui primitives copied/registered in the app |
| `commonplaceOpenUiLibrary` | OpenUI `createLibrary` registration |
| `commonplaceAntTokens` | NocoBase or any Ant-based isolated surface, pending license/product review |

No panel should hardcode the CommonPlace palette after these bridges exist.

## OpenUI Plus SceneOS

OpenUI should sit above the CommonPlace component library, not beside it. The composed agent emits OpenUI Lang. OpenUI parses and validates that output against `commonplaceOpenUiLibrary`, then renders registered React components only.

SceneOS should become a registered component family inside that library:

- `SceneArtifactPreview`: OpenUI component that receives a scene id/package descriptor and mounts the existing SceneOS web renderer.
- `ScenePackageCard`: compact provenance/status shell around a SceneOS package.
- `SceneControlStrip`: CommonPlace-skinned controls for replay, inspect, open, and fork.

This gives the agent a safe way to assemble new work surfaces while SceneOS keeps ownership of heavier scene logic. The acceptance check is explicit: an OpenUI response that names an unregistered component must fail validation; a response that names `SceneArtifactPreview` must render through SceneOS, not through ad hoc generated React.

## Migration Phases

1. **Source registry and audit**
   Add `source` metadata to each `ViewDescriptor`, land the descriptor audit, and mark the existing hand-built panels as transitional debt. Do this before changing visuals.

2. **Regime A coding harness**
   Install and adapt `react-resizable-panels`, `react-arborist`, `@codemirror/merge`/`react-codemirror-merge`, assistant-ui, and `xterm.js`. Reuse the existing `@uiw/react-codemirror` and CodeMirror/Yjs code where it already proves collaboration and theming. Replace one panel at a time so screenshots remain understandable.

3. **Allowed Theorem surfaces**
   Rebuild `AgentRunBoard`, `RunTraceTimeline`, and `ContextArtifactDrawer` over shadcn primitives and dnd-kit. These can remain bespoke because they encode Theorem concepts, but their primitives and tokens should still be sourced.

4. **Sandpack and preview runtime**
   Add Sandpack only for browser code preview/execution surfaces, not as the primary editor. Keep its Apache-2.0 license in third-party notices.

5. **Knowledge editor**
   Bring in Tiptap/BlockNote/Yjs for the compose surface and custom slash commands only after confirming the exact BlockNote packages and license obligations. The local editor stack research already favored customization-first evaluation, so verify BlockNote extension points before committing to it as the final block editor.

6. **OpenUI and SceneOS**
   Register the CommonPlace component library in OpenUI, then add the SceneOS-backed components. Do this after Regime A components exist, otherwise OpenUI will have nothing canonical to assemble.

7. **PM and records**
   Use shadcn forms plus TanStack Table for inline record metadata. Treat NocoBase as a separate research/license spike before product adoption because current licensing is not clean enough for a public extensible no-code/AI platform.

8. **Graph and map**
   Keep React Flow for close workflow/pipeline graphs. Keep cosmos.gl and deck.gl/MapLibre for large graph and geospatial descriptors when their data paths are ready.

## Recommended Package Set

Install first:

- `react-resizable-panels`
- `react-arborist`
- `@uiw/react-codemirror` (already present)
- `@codemirror/merge`
- `react-codemirror-merge` after API check
- `@assistant-ui/react`, plus the runtime package matching the first integration path (`@assistant-ui/react-ai-sdk`, `@assistant-ui/react-ag-ui`, `@assistant-ui/react-a2a`, or `@assistant-ui/react-opencode`)
- `@xterm/xterm`, `@xterm/addon-fit`, `@xterm/addon-webgl`
- `@dnd-kit/react` or the current dnd-kit React package after API check
- `@tanstack/react-table`
- `@openuidev/react-lang`, `@openuidev/react-ui`

Defer:

- NocoBase, until license/product restrictions are resolved.
- Tamagui, unless native parity becomes a near-term product requirement.
- TW-Elements, because it adds a Bootstrap-shaped third styling regime.
- ERPNext, because GPL-3.0 makes it reference-only.
- React Bits, until the exact copied component license is acceptable.
- BlockNote XL packages, unless commercial/GPL obligations are explicitly accepted.

## Acceptance Checks

- `pnpm --filter harness-console lint`
- `pnpm --filter harness-console typecheck`
- `pnpm --filter harness-console build`
- Descriptor audit: every `ViewDescriptor` declares an upstream source, mode, and regime.
- Token audit: flip one CommonPlace token and verify CodeMirror, assistant-ui, xterm, React Flow, OpenUI registered components, and shadcn surfaces update.
- Diff/editor parity: `CodeMirrorPanel` and `PatchReviewPanel` share one CodeMirror theme extension.
- assistant-ui proof: one run shows thread messages, tool calls, and an approval/action path through assistant-ui primitives.
- OpenUI guardrail proof: renderer rejects an unregistered component and successfully renders `SceneArtifactPreview` through SceneOS.
- Playwright visual proof at desktop and mobile widths for `/Commonplace`.

## Source Index

- CodeMirror docs: https://codemirror.net/docs/ref/
- React CodeMirror adapter: https://github.com/uiwjs/react-codemirror
- assistant-ui: https://github.com/assistant-ui/assistant-ui
- OpenUI: https://github.com/thesysdev/openui
- Hunk: https://github.com/modem-dev/hunk
- React Flow: https://github.com/xyflow/xyflow
- shadcn/ui: https://github.com/shadcn-ui/ui
- react-resizable-panels: https://github.com/bvaughn/react-resizable-panels
- react-arborist: https://github.com/jameskerr/react-arborist
- xterm.js: https://github.com/xtermjs/xterm.js
- dnd-kit: https://github.com/clauderic/dnd-kit
- TanStack Table: https://github.com/TanStack/table
- Sandpack: https://github.com/codesandbox/sandpack
- Tiptap: https://github.com/ueberdosis/tiptap
- BlockNote: https://github.com/TypeCellOS/BlockNote
- NocoBase: https://github.com/nocobase/nocobase
- React Bits: https://github.com/DavidHDev/react-bits
- Tamagui: https://github.com/tamagui/tamagui
- TW Elements: https://github.com/mdbootstrap/TW-Elements
- ERPNext: https://github.com/frappe/erpnext
- AionUi: https://github.com/iOfficeAI/AionUi
