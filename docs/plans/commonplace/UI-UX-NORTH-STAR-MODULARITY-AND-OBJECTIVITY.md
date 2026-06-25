# UI/UX North Star: Modularity and Objectivity Are One Architecture

Research-grounded plan for the CommonPlace / Theorem surfaces. The brief: hand-roll as little as possible, customize everything, and crack the two things that have been hard, modularity and objectivity. The finding is that they are the same architecture, that it is proven, and that your substrate already gives you the richer half of it.

## The keystone: one architecture, two names

- Objectivity is a typed object model. Everything in the product, an issue, a doc, a cycle, a file, an agent run, a patch, a scene, a claim, is an object with a type, properties, and relations.
- Modularity is three things on top of that model: views and blocks as renderers over object queries; a stable block contract so anyone can add a block or a type; and a token theme so anyone can reskin without touching logic.

They are the same mechanism from two sides. Because views render over object queries rather than bespoke screens, a new object type automatically gets every view, and a new view automatically works on every existing type. That view-over-object decoupling is the whole trick. It is what turns a pile of components into a system, and it is the layer you have been missing while choosing components.

Evidence from the landscape:

- The Block Protocol: an open standard for typed, data-driven blocks that work in any host app, defining only how a block talks to the app, not how it is built. Typed data is the convergence point.
- Schema-driven UI ("it is schemas all the way down"): one typed schema renders both the human form and the agent tool call. The shape is the single source; the UI is generated from it.
- Creatio Freedom UI: extension by replacing schemas in a hierarchy without modifying the base, so others customize without forking.
- Most directly, NocoBase, below.

## Your blueprint already exists: NocoBase

NocoBase is this architecture, shipped, Apache 2.0, TypeScript and React:

- Microkernel plus plugins. The core only does plugin lifecycle, dependency management, and base capability. All functionality is plugins, installable and removable without touching code. Like WordPress.
- Data-model-driven, with the UI separated from the data structure. The data model is the driver; blocks are the user layer; business logic bridges them. You can put multiple blocks and actions over the same object in any quantity and form. That is the object model plus views-as-renderers, in production.
- Extensible at every layer: field types, collection types, third-party data sources, middleware, blocks, and lifecycle hooks. Plugins are full-stack npm packages.
- Already agent-native. MCP, HTTP, and CLI surfaces; humans and agents work the same data model with the same fine-grained permissions; it runs with Claude Code, Codex, Cursor, and OpenCode.

So you do not need to make NocoBase modular. It already is. And you do not need to invent the modular object architecture. NocoBase proves it works and is a reference for it.

## The honest fork

NocoBase's objects are relational records. Yours are RustyRed's graph: confidence-weighted epistemic edges, H3 spatial, bitemporal facts, vectors, and the ABOUT edge from a task to any knowledge object. Your object model is strictly richer, and a relational record model cannot express it.

Therefore:

- Keep RustyRed as the object spine. Never relegate it to a second-class data source behind NocoBase's ORM, or you bury the moat under a record model that cannot hold it.
- Use NocoBase for the one surface where it is genuinely strong and tedious to rebuild: the structured-record PM and the no-code builder, with its forms, workflows, fine-grained permissions, and WYSIWYG config mode. Run it as a themed distribution of plugins, with RustyRed behind it as a data source through the PG-wire server, not as a hard fork of its core (it is already built to be extended by plugins and themes, so a hard fork throws away upstream and the plugin ecosystem you want others to build into).
- Build the surfaces NocoBase cannot, natively over RustyRed: the coding harness, the knowledge graph and its epistemic views, the composed agent, and SceneOS artifacts. NocoBase's record-and-page worldview does not fit these, and they are the moat.
- One object model on RustyRed, two surface families (the NocoBase structured-record side and the native graph/agent/artifact side), unified by a shared block contract and one token skin.

The decision to confirm: how much rides on NocoBase. Recommended split is NocoBase for structured records, PM, forms, permissions, and the no-code builder; native-over-RustyRed for the coding harness, the graph, the composed agent, and artifacts.

## The object and block contract (the layer to build first)

Define a stable block contract, in the spirit of the Block Protocol: a block receives an object query plus a type schema, renders, and emits actions. Then every surface in your component table is one kind of thing, a block over objects, and others can add blocks.

- Views (table, board, timeline, graph, card, diff, terminal, calendar) are renderers over object sets matching a shape, decoupled from types. This is NocoBase's "multiple blocks over the same record," generalized to your graph.
- Schema-driven everything. Your MCP tools already publish JSON schemas. Render object editors, forms, and config from those schemas rather than hand-building forms, so the agent-facing and human-facing definitions stay identical and you get a large surface for free.

## What you are not thinking of (the gaps)

1. The keystone is the object-and-views and block layer, not more components. Choose the contract first; the components plug in. This is the single highest-value move and the answer to both modularity and objectivity.
2. Schema-driven forms and config from your tool schemas. You already have the schemas. Stop hand-building forms.
3. Generative UI as a first-class artifact surface (thesys/openui plus SceneOS). Generative UI generates blocks and views over your objects on demand, which only works once the object model and block contract exist. Your SceneOS plus openui pairing is right; it is the payoff of the architecture, not a separate effort.
4. Skinnability is design tokens over a stable component layer, the way Obsidian themes are CSS over a stable DOM. "Let others skin over it" is a token theme layer, not forks. This is also how you get Plane's depth and 3D feel as a token-and-accent layer (the radial-orbital-timeline component, react-bits, SceneOS) rather than everywhere at once.
5. Cross-platform: tamagui (universal React Native and web, with a compiler) can unify your web and Theorem iOS component code from one source, if you want one component system across both.

## Your component list, organized into the architecture

The spine to build first:

- RustyRed object model, a block contract, a view-renderer set, and a token theme (the CommonPlace warm-paper language plus Plane-depth accents).

Knowledge editor:

- Tiptap plus Blocknote (modern, modular, slash commands). Blocksuite is dated; you are right. yrs for collaborative editing.

PM, structured records, no-code builder:

- NocoBase as a themed plugin distribution over RustyRed (PG-wire data source). TanStack Table and Query for data display. React Flow (xyflow) for workflow and graph-node close-up views. shadcn/ui and TW-Elements for custom components.

Coding harness (your next step, built native over RustyRed):

- CodeWorkspaceShell (resizable IDE layout) composing FileTreePanel, MonacoCodePanel or CodeMirror 6, TerminalPanel (xterm.js), PatchReviewPanel (Hunk, and note that diff review is the center of an agent coding tool, not the editor), AgentThreadPanel (assistant-ui, which is the thread, not the IDE), ToolActivityPanel, ContextArtifactDrawer, RunTraceTimeline, and AgentRunBoard. Sandpack for runnable preview. Every panel is a block over an object: a run, a patch, a file, a context atom.

Artifacts and generative UI:

- SceneOS plus thesys/openui (generate blocks over objects). react-bits for animated artifact components. The radial-orbital-timeline as a Theorem accent.

Skin and cross-platform:

- Design tokens for the theme. tamagui if you unify web and iOS from one component source.

## Build order

1. The object model, the block and view contract, and the token theme. Small but load-bearing; everything else plugs into it. Do it first.
2. The coding harness as the first set of blocks (your stated priority): the IDE shell, the diff-review center, the agent thread, and the run board, each a block over run, patch, and file objects.
3. PM via a themed NocoBase distribution over RustyRed (the structured-record and no-code surface).
4. The knowledge editor (Tiptap plus Blocknote) and the native graph and epistemic views (the moat).
5. Generative UI and SceneOS artifacts (the payoff of the contract).

## Grounding sources

- Block Protocol (typed, portable, data-driven blocks); schema-driven UI ("schemas all the way down"); Creatio Freedom UI (extend by replacing schemas without modifying the base); NocoBase architecture (microkernel plus plugins, data-model-driven with UI separated from data, multiple blocks over one record, extensible at every layer, MCP and agent-native, Apache 2.0, TypeScript and React). Component projects per your list: Sandpack, CodeMirror 6, Tiptap plus Blocknote, Hunk, assistant-ui, React Flow (xyflow), shadcn/ui, NocoBase, TanStack, react-bits, thesys/openui, tamagui, TW-Elements, and the 21st.dev radial-orbital-timeline.
