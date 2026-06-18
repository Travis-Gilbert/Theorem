// Shared types for the Theorem Harness Sync plugin.

/**
 * A computed semantic neighbor (a `MEMORY_SIMILAR` edge) the server may attach to a
 * doc. Distinct from authored `links` (wikilinks / `MEMORY_RELATES`): these come
 * from the substrate's kNN-over-embeddings edge builder, not from the user.
 */
export interface SimilarLink {
  doc_id: string;
  score?: number;
}

/**
 * A memory document as returned by the harness read endpoint
 * `GET /v1/tenants/:tenant/memory/docs`. `links` are the outgoing link targets as
 * doc_ids; `content_hash` is the server-computed echo gate value. `similar` is an
 * optional set of computed semantic neighbors (present only when the server
 * surfaces `MEMORY_SIMILAR` edges); it is rendered as a separate "Related" block.
 */
export interface HarnessDoc {
  doc_id: string;
  kind: string;
  title: string;
  summary: string;
  content: string;
  content_hash: string;
  status: string;
  tags: string[];
  links: string[];
  similar?: SimilarLink[];
  created_at: string;
  updated_at: string;
}

/** Response envelope of the read endpoint. */
export interface ListDocsResponse {
  ok: boolean;
  tenant: string;
  count: number;
  max_updated_at: string;
  docs: HarnessDoc[];
}

/** Arguments accepted by the `upsert_note` MCP tool. */
export interface UpsertNoteArgs {
  tenant: string;
  doc_id?: string;
  kind?: string;
  title: string;
  content: string;
  summary?: string;
  tags?: string[];
  links?: string[];
  status?: string;
  memory_node_type?: string;
  outcome?: string;
  signal?: string;
  reason?: string;
  event_id?: string;
  updated_at?: string;
}

/** Document state echoed back inside an upsert receipt. */
export interface ReceiptDocument {
  doc_id: string;
  kind: string;
  title: string;
  content: string;
  summary: string;
  status: string;
  tags: string[];
  links: string[];
  created_at: string;
  updated_at: string;
}

/** Receipt returned by the `upsert_note` MCP tool. */
export interface UpsertNoteReceipt {
  action: "created" | "updated";
  document: ReceiptDocument;
  resolved_links: string[];
  unresolved_links: string[];
  removed_links: string[];
  reconciled_back: string[];
}

/**
 * Per-doc bookkeeping the plugin persists to drive the three echo guards.
 * `contentHash` is the last server content_hash we wrote into the vault;
 * `bodyHash` is a local hash of the user-visible body we last wrote, used to tell a
 * real user edit apart from a graph-originated write.
 */
export interface DocSyncState {
  contentHash: string;
  bodyHash: string;
  path: string;
  title: string;
  updatedAt: string;
  /** Kind + summary are mirrored here so the Map-of-Content indexes can be
   * regenerated from the journal alone (an incremental pull only returns the
   * changed docs, but the index must list every synced note). Optional for
   * backward compatibility with journals written before indexes existed. */
  kind?: string;
  summary?: string;
}

/** Plugin data persisted via `saveData` (separate from user settings). */
export interface SyncJournal {
  watermark: string;
  docs: Record<string, DocSyncState>;
}

export function emptyJournal(): SyncJournal {
  return { watermark: "", docs: {} };
}
