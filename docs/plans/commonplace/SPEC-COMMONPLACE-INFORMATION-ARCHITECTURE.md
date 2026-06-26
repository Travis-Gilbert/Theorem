# SPEC: CommonPlace Information Architecture

Purpose: decide what is a page, what is an omnibar capability, what is a data view, and where every feature lives. Written so the next build organizes the product into a coherent shape instead of accreting pages. The organizing test in section 2 is the part to keep, because it places future features without another pass.

---

## 1. The mental model

CommonPlace has one spine:

- A small set of places you go to work.
- The agent everywhere, through one omnibar, with capabilities as toggles rather than rooms.
- Your RustyRed objects, seen through modular views rather than fixed screens.
- System and account configuration at the bottom, out of the way.

The product is a connective workspace where the composed agent completes knowledge work over everything you bring in, organized in an inspectable order. The information architecture serves that: few destinations, one agent surface, many lenses over the substrate.

---

## 2. The decision rule

Every feature is one of five things. Each has a one-line test. Use it to place anything new.

- **Page.** A room you go to and stay in to do a kind of work, with a working context that persists. Test: do I dwell here to work? Examples: Index, Threads, Write, Code, Artifacts.
- **Omnibar capability.** A knob on the agent or engine, used in the flow of work. Test: is this a setting on the agent, not a room? Examples: Instant KG, web search, attach, tier, git-aware, deepen, model selection.
- **Data view.** A way of looking at RustyRed objects. Test: is this a lens over my stuff? Examples: Files, Graph, Table, Map, Timeline, Clips.
- **Quick action.** A verb you fire and return from. Test: do I trigger it and leave? Examples: new note, new task, reminder, open terminal, jump to a cluster.
- **System or account.** Configuration of you, your agents, or the engine. Test: is this setup, not work? Examples: Account, Agents, Engine, Desktop, Settings.

How to place a future feature: ask the page test first. If you do not dwell there to work, it is not a page. Then ask whether it changes how the agent behaves (omnibar capability), whether it is a lens over objects (data view), whether it is a fire-and-return verb (quick action), or whether it is configuration (system). Most things that feel like they want a page are actually capabilities or views. Default against new pages.

---

## 3. The sidebar

```text
Omnibar (present on every screen)
  Ask the Theorem agent + capability toggles
Cmd-K command palette (global)

WORK
  Index         rename of Auto Organize, the triage and landing surface, default screen
  Threads        new, agent conversations tied to the work they produce
  Write          the writing destination: Notebooks, Notes, Compose as its editor
  Code           new, the coding harness
  Artifacts      generated outputs and full-canvas interactive scenes

DATA
  Files, Graph, Table, Map, Timeline, Clips
  Data is a sidebar category for lenses over RustyRed objects, not a separate page row.

TOOLBOX (collapsible, overlaps the command palette)
  See: Terminal, Cluster, Timeline
  Add: Note, Task, Reminder, Project

ACCOUNT (bottom)
  Account        profile, billing, account settings
  Agents         agent configuration, heads, bring-your-own-agent over ACP
  Engine         substrate status and configuration
  Desktop        desktop app and connectors
  Settings       app preferences

Retired: Models
```

Naming is a taste call where noted. The structure is the recommendation.

---

## 4. Page by page

**Index** (rename of Auto Organize). The default destination and triage surface. Contains Sources (Emails, Notes, Files, Tasks), Needs You (items below the confidence line that need one decision), and Organized Today (automatic filing, recent routes, where the engine put things). Day, Week, Month framing stays. This is the daily command surface.

**Threads** (new). The agent conversation home people expect. Each thread is persistent and tied to the objects and work it produced, so it reads as a record of work rather than a disposable chatbot. Returning to a thread returns to its context, its artifacts, and its run trace. This is the chat surface, framed as work.

**Write.** The writing destination. Notebooks and Notes are its contents; Compose is its editor, not a separate page. Built on Tiptap and Blocknote with yrs, with the Discovery Dock alongside. A new-note quick action covers the blank-page fast path so writing never requires navigating first.

**Code** (new). The coding harness, the priority build. This is an IDE-style workbench, not a triage page: project explorer, editor, diff review, and terminal. Agent chat should use the existing CommonPlace omnibar rather than appear as a separate permanent pane or a custom Code-only omnibar. Code-git lives here as the patch panels.

**Data.** A sidebar category for modular lenses over RustyRed objects, not a durable work page. See section 6.

**Artifacts.** Generated outputs and full-canvas interactive scenes. See section 7.

---

## 5. The omnibar and its capabilities

The omnibar, Ask the Theorem agent, is present on every screen and is the constant agent surface. It carries capabilities as toggles, not as pages.

Capabilities that live here:

- Instant KG: build a knowledge graph from the current material on the fly. A toggle, never a page.
- Web: allow the agent to browse and search.
- Attach: bring files or context into the turn. This is also the global file ingestion path, alongside drag-drop anywhere.
- Tier: the user-set difficulty that gates reasoning head count, simple, difficult, max.
- Git-aware: let the agent read and act against RustyRed git context.
- Deepen: run the heavier background passes after a save or an answer.
- Model and head selection where the user wants explicit control.

The Cmd-K command palette is the keyboard twin of the omnibar and absorbs most of the toolbox verbs.

---

## 6. Data: the modular view model

Data is the keystone of the structure and the front end of RustyRed, but it is a sidebar category rather than its own page row.

The model: a data view is a ViewDescriptor over an object query, which is the block and view contract. A view declares which object shapes it renders and which actions it emits, and the registry matches views to object sets. Add, remove, and create a view is registering or unregistering a descriptor. This is what makes the section modular: people add and subtract views and build new ones, and the agent can generate new views on demand because it composes from registered, pre-skinned renderers.

The views that ship under Data:

- Files: uploaded file objects. The RustyRed file front end. Upload is global, drag-drop anywhere plus omnibar attach, so Files is a view, not an upload page.
- Graph: the graph viewer on cosmos.gl, using the graph spec. Carries the graph-git History affordance: snapshot, diff, branch, restore the graph.
- Table: structured records. Inline tables on TanStack. The structured no-code and database builder on NocoBase, themed as its own surface in regime B per the component sourcing spec.
- Map: geospatial objects on deck.gl and MapLibre.
- Timeline: temporal view of objects.
- Clips: clipped web and media content from the Obsidian-derived clipper. This is the former Library, reframed as a data type. Pin it to the top of Data if the clipper deserves prominence.

Map and Timeline move here from the old Views group, because they are lenses over objects, not separate screens. These renderers are the same family SceneOS compiles, so Data views and generated scenes share one rendering engine.

Git in the UI: not a top-level page. Graph-git is the History affordance on the Graph view. Code-git is the patch panels in the Code harness. A unified history view across both can come later if there is demand, but it is not the entry point.

---

## 7. Artifacts and SceneOS

SceneOS is the engine, not a page. It is a trusted-component generative-UI runtime: the backend compiles a typed ScenePackage with manifest, datasets, traces, actions, provenance, renderer capabilities, patch streams, and fallbacks, and the frontend renders only registered renderers. Models never emit production React, HTML, or CSS. User actions return scene patches that preserve stable atom IDs and never silently mutate canonical graph state. Its renderers include geospatial, graph, cinematic process, mechanism diagram, comparison matrix, and image evidence.

This is the same trusted-component pattern as OpenUI, native and richer. SceneOS is the engine. OpenUI is at most a lightweight authoring and streaming-language path that plugs into the same trusted-renderer model. Both obey the rule that the agent can only assemble registered, CommonPlace-skinned components, so generated output cannot come out off brand.

What this means for the architecture:

- SceneOS powers Data views and Artifacts. The two surfaces share one renderer family.
- Generate an entire interactive interface is the agent compiling a ScenePackage into a full-canvas scene, which is saved as an Artifact after explicit confirmation.
- Artifacts is the page where generated scenes and saved outputs live and relaunch. SceneOS is how they are made.

---

## 8. Toolbox and command palette

The toolbox is a collapsible quick-action launcher, not a workspace. See holds quick jumps to views or surfaces, Terminal, Cluster, Timeline. Add holds quick creates, Note, Task, Reminder, Project. These are verbs you fire and return from.

The toolbox overlaps the Cmd-K command palette. Over time they converge: the palette is the keyboard path, the toolbox is the visible path, both invoke the same quick actions. Keep the toolbox for discoverability and let the palette carry power use.

---

## 9. Account and system

The bottom cluster is configuration, kept out of the work surfaces.

- Account: profile, billing, account settings. The labeled home for account-level configuration.
- Agents: agent configuration, the heads, and bring-your-own-agent over ACP, which is where a user plugs in their own Claude Code or other agent.
- Engine: substrate status and configuration.
- Desktop: the desktop app and connectors to outside tools, the connectivity layer.
- Settings: app preferences.

---

## 10. Retired and repurposed

**Models** is retired as a page. It was built for epistemic-era mental models of ideas, and the name now reads as AI models, which is wrong. Fold the value: structured records go to the Table view, concept and mental-model maps go to the Graph view or become a SceneOS scene type the agent can generate. Nothing in the product is called Models.

---

## 11. Open taste calls

These are yours to settle. The structure does not depend on the words.

- The name for Artifacts: Artifacts, Scenes, or Builds.
- Whether Clips stays inside Data or gets pinned to the top level for the clipper.
- Whether Threads, Index, or Code is the default screen on open.
