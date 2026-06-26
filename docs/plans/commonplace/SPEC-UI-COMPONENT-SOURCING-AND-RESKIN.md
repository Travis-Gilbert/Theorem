# SPEC: UI Component Sourcing and Reskin Map
## CommonPlace shell and Theorem coding harness

Purpose: this completes the one gap in the block-and-view contract. The contract defined the data seam (the four host methods, the three shapes) and left rendering as a verb, so the builder filled it with hand-rolled boxes. This spec binds every rendered surface to a named upstream component and defines how the CommonPlace skin lands on each. Read it as the render layer of the existing contract, not a replacement.

---

## 1. The one rule

Every ViewDescriptor binds to a named upstream component. A panel is a thin adapter that mounts an existing library component and feeds it data from the block contract. Bespoke components are allowed only where section 8 says no upstream component carries the meaning.

The agent's generative surfaces are a special case of the same rule. The composed agent does not write React. It emits OpenUI Lang, and the OpenUI renderer turns that into React restricted to a component library you register. Register the CommonPlace-skinned components as that library. The agent can then assemble new tracking and project surfaces on demand, and they cannot come out off brand, because the agent can only compose from the pre-skinned set. This is both the agent-builds-its-own-infrastructure capability and the structural fix for off-brand generative output.

---

## 2. Two theming regimes

Every surface belongs to one of two regimes. Sorting surfaces correctly is the architecture decision.

**Regime A, CSS variable and headless and Tailwind.** Components: CodeMirror 6, assistant-ui, Blocknote, React Flow, shadcn/ui, OpenUI. These read the CommonPlace token set directly and blend pixel for pixel into the editorial shell. The coding harness, the knowledge editor, the agent thread, the graph views, and the generative artifacts all live here.

**Regime B, Ant Design token.** Component: NocoBase (Ant Design 5, Formily, antd-style). It reskins to on brand through Ant's SeedToken, MapToken, and AliasToken, but its component DNA stays Ant. It lives as its own themed no-code and database surface behind the same shell, not blended into the editorial canvas.

Surfaces from the two regimes never interleave inside one view. They sit in separate routes or separate panels of the shell, each themed by its own mechanism against the same token values.

---

## 3. Adoption modes

Each binding declares one mode.

- **Vendor as is.** Install and use with default behavior, skin only. Use when the component already does the job (xterm, React Flow, CodeMirror core, cosmos.gl, deck.gl).
- **Reskin via tokens.** Use the component, restyle entirely through its theming mechanism to the CommonPlace tokens. Use for assistant-ui, Blocknote, NocoBase, CodeMirror themes.
- **Wrap and extend.** Mount the component and add a thin layer for behavior the library does not cover, such as block-contract data binding or agent annotations. Use for the diff panel inline notes, the file tree context pins, the OpenUI library glue.
- **Fork.** Copy the source into the repo and modify. Use only when a component is close but its internals block a needed change and the license permits. None required at the start.
- **Hand roll.** Build bespoke. Use only per section 8.

---

## 4. Per-surface sourcing map

### Coding harness (priority surface)

| Surface | Upstream borrow | Mode | Theming mechanism | Bespoke |
| --- | --- | --- | --- | --- |
| CodeWorkspaceShell | react-resizable-panels + shadcn | wrap and extend | CSS vars | layout glue only |
| FileTreePanel | react-arborist | reskin via tokens | CSS vars | no |
| CodeMirrorPanel | CodeMirror 6 via @uiw/react-codemirror | vendor as is + theme | CM theme extension | no |
| PatchReviewPanel (web) | @codemirror/merge MergeView, likely via react-codemirror-merge | wrap and extend | CM theme extension | agent-note overlay only |
| PatchReviewPanel (terminal) | Hunk on OpenTUI | vendor as is | Hunk custom theme | no |
| AgentThreadPanel | assistant-ui Thread, Message, Composer, ActionBar | reskin via tokens | shadcn theme, CSS vars | no |
| ToolActivityPanel | assistant-ui tool-call rendering | wrap and extend | CSS vars | no |
| TerminalPanel | xterm.js + fit + webgl addons | vendor as is | xterm theme object | no |
| RunTraceTimeline | shadcn primitives, optional react-chrono | hand roll over primitives | CSS vars | yes, light |
| AgentRunBoard | shadcn Card and Badge + dnd-kit | hand roll over primitives | CSS vars | yes, light |
| ContextArtifactDrawer | shadcn Sheet and Table | hand roll over primitives | CSS vars | yes |

For the web patch panel, @git-diff-view/react is an acceptable alternative if you want a GitHub pull-request review feel instead of the in-editor merge view. CodeMirror merge view is the default because it shares the editor engine and theme.

### Knowledge and inbox (editorial shell)

| Surface | Upstream borrow | Mode | Theming mechanism | Bespoke |
| --- | --- | --- | --- | --- |
| Compose editor | Tiptap + Blocknote + yrs | reskin via tokens | Blocknote theme, CSS vars | slash commands only |
| Discovery Dock | shadcn over block contract | hand roll over primitives | CSS vars | yes |
| Sources, Needs You, Organized Today | shadcn over block contract | hand roll over primitives | CSS vars | yes |
| Left rail, Dynamic Island, shell chrome | shadcn + Framer Motion | hand roll over primitives | CSS vars | yes, charm pieces |

### PM and structured records

| Surface | Upstream borrow | Mode | Theming mechanism | Bespoke |
| --- | --- | --- | --- | --- |
| No-code and database builder | NocoBase | reskin via tokens | Ant tokens | no, separate surface |
| Inline record metadata in editorial shell | shadcn forms + TanStack Table over block contract | hand roll over primitives | CSS vars | yes |
| Workflow and pipeline graphs | React Flow (xyflow) | reskin via tokens | CSS vars | no |

### Generative artifacts and scenes

| Surface | Upstream borrow | Mode | Theming mechanism | Bespoke |
| --- | --- | --- | --- | --- |
| SceneArtifactPreview | OpenUI renderer, @openuidev/react-lang | wrap and extend | registered library inherits CSS vars | library glue only |
| Heavier generative scenes | SceneOS + OpenUI | wrap and extend | CSS vars | scene logic |
| Hero and decorative artifacts | react-bits, radial-orbital-timeline | vendor as is | CSS vars | no |

### Large graph and map (native ViewDescriptors)

| Surface | Upstream borrow | Mode | Theming mechanism | Bespoke |
| --- | --- | --- | --- | --- |
| Large graph render | cosmos.gl | vendor as is | uniforms, CSS vars | no |
| Geospatial | deck.gl + MapLibre | vendor as is | layer props, tokens | no |

---

## 5. ViewDescriptor binding pattern

Each descriptor is an adapter. The shape, concretely:

```ts
type ViewDescriptor = {
  id: string;
  accepts: ObjectShapeMatch;        // which ObjectSet shapes this renders
  emits: ObjectActionKind[];        // which actions the user can trigger here
  render: (set: ObjectSet, host: BlockHost) => ReactNode;
};

// A panel binds an upstream component. It does not draw boxes.
const patchReviewDescriptor: ViewDescriptor = {
  id: "patch-review",
  accepts: { type: "Patch", shape: "diff" },
  emits: ["apply_patch", "comment"],
  render: (set, host) => (
    <MergeView                        // @codemirror/merge, the upstream borrow
      a={set.before} b={set.after}
      theme={commonplaceCmTheme}      // the skin
      annotations={set.agentNotes}    // wrap and extend over the block contract
      onApply={() => host.emit({ kind: "apply_patch", id: set.id })}
    />
  ),
};
```

The check the builder applies: if render returns hand-built div and border structure instead of a mounted upstream component, the binding is wrong. The only divs are layout containers from the shell.

---

## 6. Theming unification

One CommonPlace token set is the source of truth: warm amber paper background, oxblood as the sole signal color, amber focus glow, dark ink-blue composer, the type ramp, the spacing scale, the radius. It is expressed as CSS custom properties at the root.

Each regime consumes those same values through its own mechanism:

- CSS variable and Tailwind components read the custom properties directly, and the Tailwind config maps them to utility classes.
- CodeMirror reads them through a theme extension that sets editor, gutter, selection, and syntax colors from the tokens.
- xterm reads them through its theme object.
- NocoBase reads them through Ant token mapping: oxblood to colorPrimary, paper to colorBgLayout and colorBgContainer, the radius and spacing tokens to their Ant equivalents, written with antd-style createStyles.
- OpenUI inherits them, because its registered component library is built from regime A components that already read the custom properties.

No surface hardcodes a color. The black and white segmented output happened because components were drawn from scratch without the token layer. Binding to upstream components and feeding them the tokens removes that failure by construction.

---

## 7. Library status and honest corrections

| Library | What it actually is | License | Verdict |
| --- | --- | --- | --- |
| assistant-ui | headless agent-thread primitives, tool-call rendering, approvals, AG-UI and A2A and OpenCode runtimes | MIT | adopt as the agent-thread and bring-your-own-agent spine, not the code editor |
| CodeMirror 6 + @uiw/react-codemirror | extensible code editor plus React component adapter; merge view available through @codemirror/merge and react-codemirror-merge | MIT | adopt as the editor and the web diff engine |
| Hunk | terminal diff reviewer on OpenTUI, not a web component | MIT | adopt in the terminal and CLI-review path only |
| NocoBase | no-code platform on Ant Design 5, Formily, antd-style | Apache 2.0 | adopt as a separate themed PM and database surface, regime B |
| OpenUI (thesys) | generative-UI engine, OpenUI Lang plus streaming renderer plus registered component library | MIT | adopt as the agent generative render engine, the core of agent-builds-its-tools |
| Blocknote on Tiptap | block editor with slash commands, themeable | MIT | adopt as the knowledge editor |
| React Flow (xyflow) | node and edge canvas | MIT | adopt for workflows and close-up graph |
| AionUi | Electron and React cowork desktop unifying 12+ CLI agents over ACP | Apache 2.0 | reference for the connect-all-agents pattern, not a component source; a reviewer called it a thin wrapper with no moat, the trap the substrate answers |
| TW-Elements | Bootstrap-based components | free tier | do not adopt, Bootstrap is a third CSS regime that conflicts with Tailwind and shadcn; shadcn and TanStack cover these needs in regime A |
| tamagui | cross-platform styling system for React Native and web | MIT | defer unless native mobile parity becomes a goal, it is a separate styling regime |
| erpnext | ERP suite | GPL-3.0 | reference only, the license is incompatible with a proprietary product |
| react-bits, radial-orbital-timeline | animated and decorative components | MIT and component license | adopt for hero and artifact flourishes only |

---

## 8. What stays hand-rolled

Bespoke is correct only where no upstream component carries the meaning and the meaning is yours.

- **ContextArtifactDrawer**: included and excluded atoms, the token ledger, provenance. This is the Theseus context model and has no upstream equivalent. Build over shadcn primitives.
- **Discovery Dock and the Sources, Needs You, Organized Today surfaces**: these express the engine routing and confidence model. Build over shadcn primitives.
- **RunTraceTimeline and AgentRunBoard**: the run lifecycle is yours; the primitives under it are shadcn and dnd-kit.
- **The left rail, the Dynamic Island, and the warm shell chrome**: the charm pieces, where bespoke beats a library. Keep them.

The criterion: hand roll when the component encodes a Theseus or Theorem concept that no library models. Borrow everything that is a generic interface pattern.

---

## 9. Build order

Start with the token layer and the ViewDescriptor adapter pattern, because both regimes and every binding depend on them. Then bring up the coding harness in regime A, in this sequence: the resizable shell, the CodeMirror editor panel, the merge-view diff panel, the assistant-ui thread, the xterm terminal, then the file tree. These are mostly vendor as is and reskin, so the harness becomes real quickly and on brand. Then the bespoke harness surfaces that sit over shadcn: run trace, run board, context drawer. Then the knowledge editor with Blocknote and yrs. Then the NocoBase PM surface in regime B, themed through Ant tokens. Then the OpenUI generative layer, after the regime A component library exists, because OpenUI registers that library. Graph and map ViewDescriptors slot in wherever their data is ready, since they are native and self themed.

---

## 10. Acceptance criteria

- No panel renders hand-built border and box structure where an upstream component exists for that surface. Layout containers are the only bespoke divs.
- Every regime A surface reads its colors from the CommonPlace custom properties, verified by switching one token and seeing every regime A surface change.
- The NocoBase surface renders on brand through Ant token overrides, not default Ant blue.
- The agent generative output renders only registered CommonPlace components, verified by the renderer rejecting any component not in the library.
- The web diff panel and the editor share one CodeMirror theme.
- assistant-ui carries the agent thread, tool calls, and approvals, and an external agent connects through its AG-UI, A2A, or OpenCode runtime.
