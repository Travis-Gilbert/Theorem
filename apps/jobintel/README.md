# jobintel 0.1

A job-intelligence pipeline on RustyRed. It ingests the open job sources (HN
"Who is Hiring", public ATS boards), writes them into RustyRed as a graph, ranks
every role against a profile using RustyRed's own vector + graph primitives, and
emits a ranked lead queue where each lead carries a context pack ready to become
an outreach email.

Dual use: it finds contract work, **and** it is a live demo of the RAG + graph +
agent stack end to end on RustyRed (ingest, embeddings, HNSW search, PPR /
PageRank, MCP-served context packs).

jobintel is a *light client*. It talks to a running RustyRed over the public
tenant HTTP routes and never embeds the database. RustyRed runs unchanged.

## One command (no server needed)

Inspect what would be ingested, straight from the live sources:

```bash
cargo run -- ingest --dry-run
```

Prints every fetched record across all four source types (HN, Greenhouse, Lever,
Ashby) with `remote` / `contract` / `founder` flags and any in-post emails. No
RustyRed, no model download, no API keys.

## Full pipeline (ingest -> ranked queue)

Point at a running RustyRed and run the three verbs:

```bash
export RUSTYRED_URL=http://localhost:8080
export RUSTYRED_TENANT=your-tenant
export RUSTYRED_TOKEN=your-token          # omit if the server runs with require_auth=false

cargo run -- ingest                       # write Company/Role/Skill/Source/Person nodes + edges
cargo run -- rank   --profile travis      # blended semantic + graph + flag ranking
cargo run -- draft  --top 5               # write out/<company>.json packs + out/queue.md
```

`out/queue.md` is the lead index; each `out/<company>.json` is a context pack you
feed to an LLM to write the actual email. jobintel does not send.

## Configuration

| Env var | Required | Purpose |
|---|---|---|
| `RUSTYRED_URL` | yes (except `--dry-run`) | RustyRed base URL |
| `RUSTYRED_TENANT` | yes (except `--dry-run`) | tenant slug for graph scope |
| `RUSTYRED_TOKEN` | no | `Authorization: Bearer` token |
| `HUNTER_API_KEY` | no | Hunter.io key for ATS contact lookup |
| `JOBINTEL_EMBED_URL` | no | remote encoder for `--embedder http` |
| `JOBINTEL_EMBED_DIM` | no | embedding dim D (default 384) |

`slugs.toml` is the company seed list (ATS board slug + optional domain).
`profiles/<id>.toml` is your profile text + proof points.

## Embedders

The `Embedder` trait is the swap seam. Pick the backend with `--embedder`:

- `hash` (default): deterministic offline feature-hashing at D=384. No download,
  hermetic, demoable from one command. Captures lexical overlap, not semantics.
- `http`: POSTs text to `JOBINTEL_EMBED_URL` (the Theseus SBERT / any
  OpenAI-compatible `/embeddings` swap). Real semantic vectors.
- `bge`: real `bge-small-en-v1.5` (D=384) in-process via candle. Build with the
  feature: `cargo run --features bge -- ... --embedder bge`. Pulls the model
  weights from the HF hub on first use.

## How the graph is shaped

| Node | Edges |
|---|---|
| `Company` | `Company -posts-> Role`, `Role -posted_by-> Company` |
| `Role` (carries `embedding`) | `Role -requires-> Skill`, `Role -via-> Source` |
| `Skill` | `Skill -required_by-> Role` |
| `Person` | `Person -hiring_for-> Role` |
| `Profile` (carries `embedding`) | `Profile -requires-> Skill` |

The four spec edge types (`posts`, `requires`, `hiring_for`, `via`) are all
present. The reverse edges (`posted_by`, `required_by`) exist because RustyRed
PPR/PageRank adjacency is strictly `from -> to`: they let PPR mass flow
`Profile -> Skill -> Role -> Company`, which is what "warms companies whose roles
share your skills" actually requires.

## Ranking signals

`score = w_sem * semantic + w_graph * graph + w_flags * flags` (defaults
`0.5 / 0.35 / 0.15`, overridable with `--w-sem/--w-graph/--w-flags`):

- **semantic**: vector search with the Profile embedding (nearest Roles).
- **graph**: PPR seeded on Profile + its Skills (skill-relatedness) blended with
  PageRank (hiring-spike proxy: a company posting many roles ranks up).
- **flags**: `founder_posted` + `email_present` + `remote` + `contract`.

## Spec divergences (code-grounded)

These follow the *running* RustyRed server, not the spec's prose, and are noted
so the drift is visible:

- **bulk/nodes is streaming JSONL**, not a JSON array (server `LineSplitter`).
- **PPR `seeds` is `{node_id: weight}`**, not a list; PageRank takes no seeds.
- **No `GET /graph/nodes` list route** exists; readback uses
  `POST /graph/nodes/query` with `{label}`.
- **`Embedder::embed` returns `Result<Vec<f32>>`** (not the spec's bare `Vec`) so
  a failed network/model embed surfaces instead of poisoning the HNSW index.
- **Reverse-traversal edges** (`posted_by`, `required_by`) are added so the
  spec's PPR/PageRank intent works under directed adjacency.

## Tests

```bash
cargo test                    # 42 hermetic unit tests (no network)
cargo run -- ingest --dry-run # live end-to-end ingest proof
```
