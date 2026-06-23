# jobintel 0.2

A job-intelligence + outreach pipeline on RustyRed. **0.1** ingests the open job
sources (HN "Who is Hiring", public ATS boards), writes them into RustyRed as a
graph, ranks every role against a profile using RustyRed's own vector + graph
primitives, and emits a ranked lead queue. **0.2** turns that ranked backlog into
a tracked outreach loop: triage to a small daily queue, draft into Gmail, follow
up on a cadence, and learn from replies. It does not send, and it does not blast.

Dual use: it finds contract work, **and** it is a live demo of the RAG + graph +
agent stack end to end on RustyRed (ingest, embeddings, HNSW search, PPR /
PageRank, MCP-served context packs).

jobintel is a *light client*. It talks to a running RustyRed over the public
tenant HTTP routes and never embeds the database. RustyRed runs unchanged. The
outreach engine adds one more HTTP seam (the Gmail API), with the same
base-URL-swappable, mock-testable shape as the RustyRed client.

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

## Outreach engine (0.2)

The 900 ranked roles become a backlog worked at a sane daily volume. The outreach
loop is five verbs over the same graph, plus a Gmail create-draft bridge:

```bash
export GMAIL_TOKEN=...          # operator's Gmail OAuth access token, or a path to a file holding it
# DAILY_DRAFT_CAP=8  FOLLOWUP_DAYS=4,9   (defaults shown)

jobintel outreach queue              # today's work: to-draft, drafted-not-sent, follow-ups due
jobintel outreach draft --top 8      # draft the top queued leads into Gmail (capped); never sends
jobintel outreach sync               # detect replies + advance drafted -> sent
jobintel outreach followups          # draft the next nudge for leads past their date; reap the exhausted
jobintel outreach stats              # reply rate per template and per lead type

# operator helpers (beyond the four core verbs):
jobintel outreach trail --role role:hn:123      # reconstruct a lead's event trail from the graph
jobintel outreach mark  --role role:hn:123 --status dead   # manual override
```

The loop is autonomous through `sync`: jobintel writes Gmail **drafts**, the
operator sends them with one click, and `sync` detects the send (the draft leaves
the drafts list while its thread keeps a message) to advance `drafted -> sent` and
schedule the first follow-up. Reply detection (a lead address appears in the
thread) flips the lead to `replied` and stops the follow-ups.

State lives on the `Role` node (`outreach_status`, `touch_count`,
`next_followup_at`, `gmail_draft_id`, `gmail_thread_id`, ...) plus an append-only
`OutreachEvent` trail (`Role -has_outreach-> OutreachEvent`), so the whole history
reconstructs from the graph. Terminal outcomes (`replied`/`dead`) write an
`OutcomeRecord` that `stats` aggregates.

Lead type drives both the template and the stats grouping:

| Lead type | When | Template |
|---|---|---|
| `contract_explicit` | the post wants contract/freelance (`contract` flag) | `templates/outreach/contract_explicit.txt` |
| `hn_founder` | an HN "Who is Hiring" lead | `templates/outreach/hn_founder.txt` |
| `ats_role` | a Greenhouse/Lever/Ashby role | `templates/outreach/ats_role.txt` |

The **guardrail** is load-bearing, not a throttle: `DAILY_DRAFT_CAP` plus the
two-nudge ceiling (day 4, day 9, then stop) keep volume in the range where
personalization, not blast, does the work. Cold mail from a personal domain
degrades sender reputation past a few dozen a day.

## Configuration

| Env var | Required | Purpose |
|---|---|---|
| `RUSTYRED_URL` | yes (except `--dry-run`) | RustyRed base URL |
| `RUSTYRED_TENANT` | yes (except `--dry-run`) | tenant slug for graph scope |
| `RUSTYRED_TOKEN` | no | `Authorization: Bearer` token |
| `HUNTER_API_KEY` | no | Hunter.io key for ATS contact lookup |
| `JOBINTEL_EMBED_URL` | no | remote encoder for `--embedder http` |
| `JOBINTEL_EMBED_DIM` | no | embedding dim D (default 384) |
| `GMAIL_TOKEN` | yes (outreach draft/sync/followups) | Gmail OAuth access token, or a path to a file holding it |
| `DAILY_DRAFT_CAP` | no | max drafts per `outreach draft` run (default 8) |
| `FOLLOWUP_DAYS` | no | days-after-send for nudges (default `4,9`) |
| `JOBINTEL_GMAIL_API` | no | Gmail API base URL (default `https://gmail.googleapis.com`; swap for a mock) |

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
| `OutreachEvent` (0.2, append-only) | `Role -has_outreach-> OutreachEvent` |
| `OutcomeRecord` (0.2, terminal) | `Role -has_outcome-> OutcomeRecord` |

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

### 0.2 (outreach) divergences

- **Status updates are read-modify-write, not a partial patch.** RustyRed's node
  upsert *replaces* a node wholesale (no merge, no patch/CAS route), so setting
  `outreach_status` GETs the full Role and re-upserts it with title/body/embedding
  preserved. A naive `{id, outreach_status}` upsert would wipe the role.
- **`drafted -> sent` is detected, not commanded.** jobintel never sends, and the
  CLI has no "mark sent" verb, so `sync` infers a send from Gmail state: the draft
  has left the drafts list while its thread still holds a message. The send date is
  taken as the sync date.
- **Drafts are rendered deterministically.** The spec says "the model fills the
  per-lead specifics"; jobintel has no model in the loop, so it renders a complete,
  editable draft by substituting the role's own language (a skill-bearing sentence
  from the post) plus the fixed proof block. The operator edits before the
  one-click send.
- **`contract_explicit` takes precedence in lead typing.** The spec lists the three
  types without precedence; for an operator seeking contract work, a contract post
  gets the contract template even when it is also an HN/founder post.
- **`set_status` is realized as a bundled read-modify-write.** The Module 1 deliverable
  exists (`state::set_status`), but the live transitions bundle status + cadence
  fields into one upsert so a touch is one graph write.
- **Two extra operator verbs beyond the spec's four:** `outreach trail` (reconstructs
  the event trail from the graph - the Module 1 acceptance) and `outreach mark` (manual
  status override). The core loop is still queue/draft/sync/followups (+ stats).
- **`touch_count` counts sent touches:** 0 at draft, 1 at send, +1 per nudge. The
  day-4/day-9 schedule is anchored to the send date (`outreach_sent_at`), and a
  non-replying lead is reaped to `dead` one day after the final interval.

## Tests

```bash
cargo test                    # 139 hermetic tests (75 unit + 6 + 58 integration; no network)
cargo test --test outreach_seams  # Gmail draft seam + read-modify-write proofs against a mock server
cargo run -- ingest --dry-run # live end-to-end ingest proof
cargo check --features bge    # local-encoder path compiles
```

The outreach loop has no live-network proof in this build (it needs the operator's
real Gmail token + a running RustyRed); the seams are proven against mock servers,
the way 0.1's HTTP contract is.
