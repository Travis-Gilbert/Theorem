# Block and View Contract (North Star)

The keystone layer under everything: the stable seam that makes the coding harness, the PM surface, the knowledge editor, and SceneOS artifacts the same system. It is small on purpose. The entire modularity and objectivity promise rides on this seam staying still while everything above it changes.

## The one idea

A block receives a host with four methods and nothing else:

```ts
type BlockHost = {
  query(q: ObjectQuery): ObjectSet
  emit(a: ObjectAction): Promise<Result>
  viewsFor(shape: ObjectShape): ViewDescriptor[]
  tokens: ThemeTokens
}
```

A block never touches RustyRed, the harness, the router, or the theme directly. It asks the host for objects, it emits intents, it asks which views fit a shape, and it reads tokens to paint. That is the whole contract surface. Keep these four stable and the three shapes below stable, and any view, type, or panel anyone writes composes without coordination.

## The object model the three shapes reference

Every thing is an Object: an id, a Type, typed properties, typed relations (graph edges), and the axes it happens to have (spatial H3, bitemporal time, a vector embedding). A Type is a schema. Your CommonPlace Item is already one instance of this (kind, title, body, tags via HAS_TAG, collections via IN_COLLECTION, embedding, status, priority, due_at, and the SUBTASK_OF, DEPENDS_ON, ABOUT, WORKED_BY edges). The contract generalizes that pattern; it does not replace it.

```ts
type TypeDef = {
  name: string
  properties: { name: string; type: PropType; constraints?: Constraint[] }[]
  relations: { edge: string; dir: "in" | "out"; target: TypeRef }[]
  axes: { spatial?: boolean; temporal?: boolean; embeddable?: boolean }
}
```

## 1. The object query shape (read)

A serializable, typed description of an object set. It is not raw SQL; it carries the substrate's multimodal reach so a view can ask for graph, vector, fulltext, space, and time in one shape.

```ts
type ObjectQuery = {
  types: TypeRef[]
  where?: Predicate
  traverse?: EdgeWalk[]
  rank?: Ranker[]
  fuse?: FusionPolicy
  slice?: { valid?: TimeRange; tx?: TimeRange; space?: H3Window }
  project?: Projection
  page?: { limit: number; cursor?: string }
  live?: boolean
}

type ObjectSet = {
  objects: ObjectRef[]
  shape: ObjectShape
  subscribe(cb): Unsubscribe
}
```

Grounding: ObjectQuery compiles to your native QueryIr (relations, predicates, joins, projection, limit, fusion, with knn, textmatch, and expand as rankers), or to the GraphQL selection AST for simple typed reads (items, neighbors, vectorHybrid, fulltextSearch, spatialRadius). The block speaks ObjectQuery; the host compiles. The `shape` on the result is what the registry matches views against, so query and render stay decoupled.

## 2. The action protocol (write and invoke)

Actions are declarative intents, values not function calls, so the host can permission-check, stamp provenance, log, and undo them, and so a human and an agent head emit the same intents through the same path.

```ts
type ObjectAction =
  | { kind: "create"; type: TypeRef; props: object }
  | { kind: "update"; id: string; patch: object }
  | { kind: "delete"; id: string }
  | { kind: "link"; from: string; edge: string; to: string; confidence?: number }
  | { kind: "unlink"; from: string; edge: string; to: string }
  | { kind: "run_agent"; target: ObjectRef | ObjectQuery; tier: "simple" | "difficult" | "max" }
  | { kind: "invoke_tool"; tool: string; args: object }
  | { kind: "dispatch"; job: JobSpec }
  | { kind: "open"; id: string; view?: string }
  | { kind: "select"; ids: string[] }
```

Grounding: mutations resolve to graphql_mutate (designate and bulk writes, edge operations) and upsert_note where wikilink edge reconciliation applies. run_agent resolves to composed_agent_run through the binding scratchpad and alignment gate. invoke_tool resolves to invoke or tool_search over the federated affordances. dispatch resolves to the job board and spawn_session. Provenance is the actor, or the actor_head_id for an agent head, stamped on emit; permission is the alignment gate and role model, enforced on emit. The block emits the same value whether a person or a head triggered it.

## 3. The view registry (match)

```ts
type ViewDescriptor = {
  id: string
  name: string
  accepts: ObjectShapeMatch
  emits: ActionKind[]
  render: (set: ObjectSet, host: BlockHost) => UI
}
```

The host matches: `viewsFor(set.shape)` returns the descriptors whose `accepts` is satisfied. A new type automatically gets every view it matches; a new view automatically works on every type whose shape it accepts. That is the modularity, mechanically.

Three things plug in here without special cases:

- Generative views register at runtime, an `accepts` shape plus a `render` produced by thesys or openui. Same descriptor, generated render.
- The coding harness panels are descriptors, not bespoke screens. AgentRunBoard accepts many AgentRun. PatchReviewPanel accepts one Patch. FileTreePanel accepts File over the CONTAINS edge. They register the same way anyone else's panel would, which is why the harness composes in a shell alongside everything else and stays open to extension.
- The NocoBase structured-record blocks sit behind this contract too, exposed as views over the same objects, so the NocoBase surface and the native surface read one object model.

## What is not in the contract

Layout (how blocks are arranged in a workspace) is the shell's job, CodeWorkspaceShell and its siblings, not the contract. Skin is tokens, passed in, not the block's concern. Auth, provenance, and undo are the host's job, enforced on emit, uniform for human and agent. Keeping these out is what keeps the contract small enough to stay stable.

## The one real dependency to satisfy

The contract requires a live binding: an ObjectSet that updates as objects change. For an object's editable body (the knowledge editor) this is already the CRDT and streaming layer you just built, a yrs region. For record sets (a board, a run list) it needs a query subscription or change feed at the same granularity. Confirm whether that record-level subscription exists yet, or extend the streaming layer to carry it, or fall back to a lightweight poll for record sets in the interim. This is the single thing the host must provide that may not be fully in place; everything else maps to tools that exist.

## Worked example, a Task board

Query: types [Task], where status not done, traverse ABOUT to surface linked knowledge, rank due_at ascending, live. The result shape is Task, with a temporal axis (due_at) and edges. `viewsFor` returns table, board, card, timeline, graph. The user picks board. Dragging a card emits `{ kind: "update", id, patch: { status } }`. The host resolves it to graphql_mutate, checks permission, stamps provenance, the live ObjectSet updates, the board re-renders. The same board works on any type with a status-like field and a temporal axis, because it matched on shape, not on Task.

## Worked example, the coding harness PatchReviewPanel

Descriptor accepts one Patch. Query: types [Patch], where run equals the current run, project the diff, the file refs, and the agent notes, live. It renders a side-by-side diff. Approve emits `{ kind: "dispatch", job: applyPatch }` or `{ kind: "run_agent", target: patchRef, tier }`, resolved through dispatch or composed_agent_run. The panel is a registry entry, so it drops into CodeWorkspaceShell next to the file tree, the terminal, and the agent thread, and a contributor adds a new panel the same way, by registering a descriptor.

## Stability

Versioned. The stable surface is the four host methods and the three shapes (ObjectQuery, ObjectAction, ViewDescriptor with TypeDef). Which views exist, which types exist, how they render, how they are laid out, and how they are skinned are all free to change without breaking a block. Churn in this seam breaks every block at once, so it changes rarely and on a version boundary. That discipline is the price and the point of the contract.
