# Harness Console: Libraries, Design Math, Depth, and Components (v2)

Consolidated addendum to SPEC-harness-console-surface.md. This version folds in the confirmed decisions and the new requirements: the starting palette, the build phasing, the Dynamic Island TOC with Civic Atlas ambient logic and RustyWeb search, the depth system grounded in your site, and the GitHub plus MCP hub settings page. Where this differs from the prior addendum (palette, phasing), this v2 supersedes it.

Format: named choices are requirements. No mandate blocks, no build-order tables, no time estimates, no em dashes.

## Build context, read first

This is a new standalone project. A new Vercel project on the subdomain harness.theoremsweb.com, built greenfield. Do not open, extend, or inherit Context-Theorem-UI, the existing control plane at theoremweb.com. Do not pull its shell, its components, or its styling. Its design is explicitly not the target. The reason this is a new project is a clean design that does not carry those primitives, so building on top of the existing app is the one thing to avoid.

The only shared substrate is the harness backend: the harness MCP tool surface, the memory graph, and the connector gateway. The new console consumes that backend over MCP and HTTP and does not rebuild it. Everything the user sees is built fresh in this project.

The surfaces are defined in the surface spec and built new here: Agent, Memory, Skills, Rooms, Runs, API Keys, Providers, Usage, Connections, and Settings. The information architecture can resemble the existing product because the product needs the same surfaces, but no page, component, or token is carried over from the existing app.

One thing is borrowed, and as technique rather than look: the depth craft from travisgilbert.me, the DotGrid canvas, the materiality layering, and the elevation scale, re-tokenized to the palette below. That is a personal-site technique worth keeping, not a control-plane primitive, and it arrives recolored into the new system rather than copied.

## Build phasing

Your call to start simple and grow is the right one, and it lowers the design risk you flagged. Three phases by order, not by time.

Phase 1, structure. Stand up the new project with shadcn and Radix primitives and a fresh token foundation in the palette below. Get the shell, the rail, the routes, and every page in place with real navigation and mock data. No RetroUI, no heavy treatment, no depth flourishes, and nothing pulled from the existing app. The only goal is that the information architecture and the layout are correct.

Phase 2, depth and the spine. Add the depth system (ambient field, surface materiality, elevation scale) and the Dynamic Island TOC. This is where the console stops looking like a wireframe and starts feeling like an instrument.

Phase 3, treatment and the hard components. Retokenize RetroUI components in for the instrument-brutalist surfaces, then build the memory cluster, the collaborative IDE, and the Connections and MCP Hub page. These are the parts that benefit from the structure being settled first.

## Starting palette

Supersedes the warm-paper palette for the start. You can add color from here.

- Background: white. If pure white glares in the editor, warm it a hair, but start white.
- Surfaces: grey. The sidebar, cards, and panels sit on a light grey so they read as surfaces against the white field. Two or three grey steps, no more, at the start.
- Outlines: black hairlines, 1px. This is the structural line that does the work the rough.js border did on the site, but clean rather than hand-drawn.
- Accent: oxblood. One accent, used for the active nav item, the primary action, the focus ring, and the active search toggle. You have used two oxblood hexes, #A8301E on the Theorem and iOS side and #6b2c33 on OurCivicAtlas; pick one for the console accent and make it the token. Default suggestion is #A8301E unless you want the deeper one.
- Shadows: grey, soft, layered. Neutral rather than warm-brown, because the field is white now.

Tokens: --bg (white), --surface and --surface-2 (grey steps), --line (near-black hairline), --ink (near-black text), --muted (grey mid for metadata), --accent (oxblood), and the elevation shadow tokens in the depth section. Contrast still holds at 4.5:1 for body and 3:1 for large text, UI, and borders.

## Renderer model

The console picks its own renderers and is not bound to the existing app's renderer or its look. Two lanes by architecture, not by inheritance.

Lane A, the GPU lane: cosmos.gl for very large graphs. The memory and knowledge-graph view is the canonical Lane A case, and with RustyRed going multi-model it is the multimodal cluster.

Lane B, scenes and charts: D3 for force layouts, hierarchies, geo, sankey, contour, scale, and shape, and for one-off charts. If consuming the existing Scene OS renderer, a self-contained D3 canvas that takes a scene-package-v2, saves real work for a given scene, it can be used as an engine dependency, but that is a choice rather than a default, and it brings no chrome and no design with it.

Rule for any new graph surface: cosmos.gl when node count is large, D3 otherwise, Scene OS only where it earns its place as a pure renderer.

## Library stack

- D3. The Scene OS house renderer. Use it through scene-package-v2 for scenes, and directly for the cluster projection math and one-off charts.
- cosmos.gl. The GPU lane for the memory cluster and any future very-large graph.
- deck.gl. Only if memory or results nodes render as image thumbnails rather than dots or cards, since cosmos.gl does not do per-node sprites. Dots and cards do not need it.
- Anime.js v4. Motion, consistent with the Theorem launch work: drawable SVG lines, staggered reveals, ScrollObserver, built-in reduced-motion scope.
- motion (motion/react). The Dynamic Island TOC uses it for the pill expand and the spring entrance. Keep it for that component family; do not duplicate its role with anime.js.
- shadcn and Radix. The accessible primitive layer for tables, dialogs, tabs, command palette, and toasts. This is the Phase 1 base, wired with the starting tokens.
- RetroUI. A neo-brutalist set that installs as a shadcn registry, so it composes with shadcn and inherits Radix accessibility. Phase 3. Adopt the components and retokenize to the palette and type system: oxblood and black borders rather than neon, grey and white fills, Vollkorn and IBM Plex rather than pixel or CRT fonts, the offset shadow kept. The structural boldness fits the Field Notes lineage; the loud defaults do not.
- Yjs. Already in the harness. The CRDT substrate and the source of truth for collaborative documents. Document ownership stays here.
- CodeMirror 6 with y-codemirror.next. The editor for Skills and code, bound to Yjs. Markdown mode for memory atoms reuses the same component.
- Velt, cursors specifically. Not wholesale. Take the collaboration components, above all the live cursors, and render them over a Yjs doc that Velt does not own. The cursors are the part that is hard to make look right and the part you want; the data plane stays on your Yjs.

## The 4x4 design math

The explicit math of the system, four axes, each constrained to a small fixed set, each with a rule that can be checked rather than judged by eye. This is the consolidated form of the design math from design-pro and the design-engineering corpus, gated the same way that skill is: render a component, run the token lint and axe, pass or fail.

Discipline in one line: tokens before pixels. Every value on every axis comes from its scale. Nothing freehand.

### Axis 1, Space

Base unit 4px. Full scale: 4, 8, 12, 16, 24, 32, 48, 64. The four everyday steps: 8 inside a control, 16 between elements, 24 between sections, 32 for major breaks. Tokens --space-1 through --space-8. Rule, checkable: every margin, padding, and gap resolves to a space token; a linter flags raw px in those properties.

### Axis 2, Type

One ratio, 1.25. Ramp, rounded: 12 for labels and captions in mono, 15 body, 19 subhead, 24 title, 30 display and rare. The four everyday sizes are 12, 15, 19, 24. Roles: Vollkorn serif for titles and human headings, IBM Plex Mono for labels, metadata, status, and the omnibar, IBM Plex Sans Condensed for dense body and data. Rule, checkable: sizes come from the ramp; body is at least 15 with line-height at least 1.4; running text measure stays 45 to 75 characters.

### Axis 3, Color

One accent, oxblood. A neutral value ramp from white through grey to near-black ink. One status color, a calm green. Distribution 60-30-10: about 60 percent white and grey surface, 30 percent secondary neutral and structure, 10 percent accent. The accent marks one thing per region. Rule, checkable: every color is a token; body meets 4.5:1, large text and UI and borders meet 3:1; contrast is computed, not eyeballed.

### Axis 4, Hierarchy

Hierarchy is the output of the first three axes. At most four levels of emphasis per screen. Each level differs from the next on at least two of size, weight, space, and color, never one alone. One primary action per region. Related items grouped by proximity, so whitespace groups before borders do. Rule, partly checkable: one primary action and a clean heading order are checkable; whether the squint test passes is the judgment you keep.

### Enforcement

The four axes ship as tokens in one file. A token and scale linter checks axes 1, 2, and 3. axe checks contrast and the accessibility parts of axis 4. Same render-and-check spine as the design-engineering skill, so the console doubles as a fixture set for it.

## Depth

You named depth as the thing to do better, and pointed at your site. Reading the site, your depth is three concrete techniques, and none of it is 3D.

The canvas is DotGrid.tsx: a fixed full-viewport dot field where a seeded PRNG (mulberry32) deterministically turns about a fifth of the dots into tiny 0s and 1s for a digital texture, with mouse repulsion, spring-back, and a decaying ink trail. PWRN reads as PRNG, that seeded binary scatter.

The compute fade is the computeFade function: a kite-shaped edge vignette that is full at the top and tapers at the sides and bottom, or an inverse vignette that keeps the center clean for reading and fades dots in toward the edges, plus the top inversion gradient where dots flip to cream under the dark header and fade out through a soft tail.

The materiality layer from your surface doc sits on top: opaque surface fill, a 40px blueprint grid at 0.15, an feTurbulence paper grain at 0.03, warm brown shadows rather than gray, and a hover-lift that feels like lifting a document off a stack.

Ported to the console, depth is three layers plus an elevation scale that makes it checkable.

Layer 1, the ambient field. Reuse DotGrid as the page background. Use the inverseVignette mode so the work area stays clean and dots fade in toward the edges. Recolor the dots grey for the white scheme through the CSS variables it already reads. Keep the reduced-motion path it already has. This is the single biggest depth win and it already exists as a reusable component.

Layer 2, surface materiality. Cards are opaque grey surfaces with a faint grain, an optional 40px blueprint grid, a two-layer soft shadow, and a hover-lift of one pixel with the shadow growing. Retokenize the warm-brown shadows to neutral grey for the white field. Drop rough.js for the console and use the clean black hairline instead; the hand-drawn border is a site signature, and the console reads cleaner without it.

Layer 3, the elevation scale, which is the math of depth. Four levels, each a defined shadow token and a z-index band, the same discipline as the spacing scale. flat is level 0, no shadow, on the field. raised is a card, a tight shadow plus a soft wider one. floating is the Dynamic Island and popovers, a larger soft shadow. overlay is modals and the expanded-TOC backdrop, the largest shadow plus the backdrop blur. Every elevated surface uses an elevation token, never a freehand shadow. Tokens --elev-0 through --elev-3. This makes depth quantized and checkable rather than something you tune by hand each time.

Optional flourishes, not the system. The Glowing Shadow card, an animated glow border driven by CSS @property, tamed to oxblood, works as a single hero accent on one element and nowhere else. The animated wave footer is a motion flourish for the footer. Use both sparingly; the depth system is the three layers and the elevation scale.

## Component: The Dynamic Island TOC and omnibar

One element, bottom-center, a permanent fixture in the bottom third of the page, built on the Dynamic Island TOC pattern (motion/react, the pill that expands with a backdrop). It is the omnibar, the command spine, the table of contents, the ambient context bar, and the RustyWeb search box, unified by the Dynamic Island metaphor, which also matches the Dynamic Island control surface locked for the iOS app. It carries the visual treatment of the reuno-ui ai-input: a rounded pill with a soft border, an attach affordance, a search toggle, and a send, retokenized to white, black, grey, and oxblood, with oxblood for the active search-toggle and send states rather than the demo accent.

State, ambient (collapsed pill). The bar inherits the most relevant context of the current surface. This is the Civic Atlas logic: the bar reflects the most salient thing on screen. By default that is the active section, tracked by scroll-spy the way the component tracks the active heading, with a circular progress ring. On a graph surface, hovering a node overrides the ambient content to show that node's title and a little metadata, its kind, confidence, or edge count, whatever the node carries. Move off the node and the bar returns to the active section.

State, expanded (on click). The full list, over a backdrop blur at overlay elevation. On a content surface it is the table of contents, read from data-toc, data-toc-title, data-toc-depth, and data-toc-ignore. On the memory surface it is the cluster list, which is the same set of floating cluster labels from the memory cluster component.

State, search. The pill is also a RustyWeb search box. The search toggle is the connection indicator: when it is on, the bar is wired to RustyWeb, the way the Globe toggle signals web search in the ai-input. Submitting a query does not open a SERP. Results render as the expanded TOC view of results, a ranked list inside the island's expanded panel, or as a results graph, a cosmos.gl or Scene OS results cloud in the recent.design style, with a toggle between the two. Search reuses the same two surfaces the rest of the app uses, a list or a graph, rather than introducing a third results format.

Pressing the command key expands the bar into the full command palette from any surface, and escape returns it to the resting ambient pill.

Acceptance:
- The ambient pill shows the active section with a progress ring on a content surface.
- Hovering a graph node shows that node's title and metadata in the pill; leaving it restores the section.
- Clicking expands to the TOC on a content surface and to the cluster list on the memory surface, over a blurred backdrop.
- The search toggle visibly indicates the RustyWeb connection, and a query returns either a ranked TOC list of results or a results graph, never a SERP.
- The command key expands the bar from any surface; escape collapses it.

## Component: Memory cluster graph

The recent.design look, applied to memory. Lane A.

Pipeline: embed each atom, which already carries an embedding, so no new embedding step for text atoms; cluster with the harness community detection that already exists (Leiden or label-propagation), each cluster named; project the embeddings to 2D with UMAP, computed server side and stored as x and y on each atom; render with cosmos.gl as points or small cards at their coordinates, colored by cluster, or with deck.gl as thumbnails if atoms carry images.

Interaction: cluster labels float to the side as a quiet list and are the navigation, which is the same list the Dynamic Island shows when expanded on this surface; clicking a label frames that cluster; clicking a node opens the same markdown editor used everywhere in Memory, because the graph is a projection of the truth and editing edits the truth; zoom is the density control, the blob and labels far out, individual atoms and titles close in.

States: empty prompts to ingest rather than showing an empty canvas; a clear computing state while UMAP runs or hydrates; a single-cluster focus state after a selection.

Acceptance: atoms render at stable positions grouped and colored by community with floating labels; a label frames its cluster and a node opens its editor; the view stays interactive at current scale and ten times it, with detail gated by zoom.

## Component: Collaborative agent IDE

The human and a harnessed agent editing the same document live. On the Skills and builder editors first, available to any code or markdown surface.

The document is a Yjs doc and Yjs is the source of truth. The human edits through CodeMirror 6 bound with y-codemirror.next. The agent is a participant, not a separate channel: a server-side Yjs client applies the agent's edits into the same doc with its own awareness identity, so it appears with its own cursor and selection. An agent rewriting a function shows up as a second cursor moving through the file, not as a diff dropped in afterward. Presence and live cursors come from Velt's collaboration components rendered over the Yjs doc, with comments either from Velt or from Yjs awareness; Velt does not own the document.

Visual treatment: the editor sits in the calm content zone with muted warm syntax colors rather than a saturated dark theme, remote cursors in each participant's color, and a retokenized RetroUI frame so it reads as a deliberate bordered surface rather than a raw textarea.

Acceptance: two participants, one an agent, edit the same file and see each other's cursors and edits live; the document state lives in Yjs and survives without the presence layer; the agent's edits arrive as live cursor movement attributed to the agent.

## Surface: Connections and MCP Hub

A new page under Settings. Two parts.

GitHub connection. Connect GitHub through the existing connector path, authorize repositories, and the harness ingests the selected repos into the code graph, which is the GitHub App ingestion already specified as a prior handoff. The page shows connected repos and their ingestion status. This part is grounded in prior specs.

MCP hub. The harness is the single MCP endpoint your coding agents connect to. It aggregates the harness's own capabilities, exposed as namespaced verbs through gRPC reflection per the prior Workstream E, and brokers other MCP servers, re-exposing them through the one harness connection so a client like Claude Code wires up once and reaches everything behind it. This page manages what the hub exposes: toggle capability namespaces on or off, register or remove brokered MCP servers, and copy the single connection snippet per client, the same per-client snippets as the API Keys surface. This is the protocol-level answer to too many front doors: one connection in Claude Code, many capabilities behind it.

Honesty note. The GitHub connection and the verb-exposure mechanism are grounded in prior specs, the GitHub App ingestion handoff and the Workstream E gRPC-reflection-to-verbs work. The capability to broker arbitrary external MCP servers is the natural extension of the hub thesis and the too-many-front-doors brief, specified here as a surface. If the recent spec you are recalling has specific mechanics for the broker, point me to it and I will align this surface to it.

Acceptance: connecting GitHub shows repos ingesting; toggling a capability namespace changes what the hub exposes; registering a brokered MCP server makes it appear in a client through the single harness connection; each client has a one-connection snippet to copy.

## What did not change

Everything in the surface spec this addendum does not touch still holds: the two-key model, the onboarding flow modeled on Browser Use, the per-client install snippets, the Providers surface resolving credential references, the Memory list as the primary view with archive and trash, and the definition of done. This v2 sets the starting palette, the build phasing, the depth system, the Dynamic Island TOC with Civic Atlas ambient logic and RustyWeb search, and the Connections and MCP Hub surface, and it keeps the 4x4 math, the renderer model, the memory cluster, and the collaborative IDE from v1.
