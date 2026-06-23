import { App, Notice, TFile } from "obsidian";
import type { HarnessClient } from "./harness";
import { isCommonsTenant, type HarnessSyncSettings } from "./settings";
import type { SyncJournal, UpsertNoteArgs } from "./types";
import { localHash } from "./hash";
import { extractWikilinks, isCaptured, userBody } from "./notes";
import { GENERATED_FRONTMATTER_KEY } from "./indexes";
import type { SyncGuard } from "./guard";

const ENCODE_KINDS = new Set(["encode", "feedback", "solution", "postmortem"]);

/**
 * The write-back half (Phase 2). A note that carries a `doc_id` round-trips as an
 * update; a new note in the capture scope becomes a new doc. Wikilinks become
 * link targets the harness reconciles into edges. Note-linking is graph construction.
 */
export class WriteBack {
  /** Tracks whether the commons-tenant block has been surfaced once this session. */
  private commonsWarningShown = false;

  constructor(
    private app: App,
    private client: HarnessClient,
    private settings: HarnessSyncSettings,
    private journal: SyncJournal,
    private guard: SyncGuard,
    private save: () => Promise<void>
  ) {}

  /** Returns true if the note was pushed; false if it was out of scope or unchanged. */
  async handleChange(file: TFile): Promise<boolean> {
    if (!this.settings.enableWriteBack || file.extension !== "md") {
      return false;
    }
    if (this.guard.isSuppressed(file.path)) {
      return false;
    }

    const cache = this.app.metadataCache.getFileCache(file);
    const frontmatter = (cache?.frontmatter as Record<string, unknown> | undefined) ?? null;

    // Plugin-owned generated index notes carry the generated flag; never push them.
    if (frontmatter?.[GENERATED_FRONTMATTER_KEY]) {
      return false;
    }

    const existingDocId = asString(frontmatter?.doc_id);

    // A note already bound to a doc always round-trips; a new note must be captured.
    if (!existingDocId && !isCaptured(file.path, frontmatter, this.settings)) {
      return false;
    }

    const text = await this.app.vault.read(file);
    const body = userBody(text);

    // Echo hash gate: a body matching what the graph last wrote never pushes.
    if (existingDocId) {
      const state = this.journal.docs[existingDocId];
      if (state && state.bodyHash === localHash(body)) {
        return false;
      }
    }
    if (!body) {
      return false;
    }
    // This note would push. Never let hand-written notes land in the commons
    // ("default") tenant unless the user opted in. Surface the block once.
    if (this.isCommonsBlocked()) {
      return false;
    }

    const args = this.buildArgs(file, frontmatter, body, existingDocId);
    const receipt = await this.client.upsertNote(args);
    const docId = receipt.document.doc_id;

    if (!existingDocId) {
      await this.writeDocIdBack(file, docId);
    }

    this.journal.docs[docId] = {
      contentHash: this.journal.docs[docId]?.contentHash ?? "",
      bodyHash: localHash(body),
      path: file.path,
      title: args.title,
      updatedAt: receipt.document.updated_at,
      kind: args.kind,
      summary: args.summary ?? "",
    };
    if (receipt.document.updated_at > this.journal.watermark) {
      this.journal.watermark = receipt.document.updated_at;
    }
    await this.save();
    return true;
  }

  /**
   * A vault delete tombstones the bound doc. The file is already gone, so the
   * metadata cache is unreliable; the doc_id is resolved from the journal by path.
   * Returns true if a doc was tombstoned. The `guard` suppresses plugin-driven
   * deletes so a tombstone never echoes.
   */
  async handleDelete(path: string): Promise<boolean> {
    if (!this.settings.enableWriteBack) {
      return false;
    }
    if (this.guard.isSuppressed(path)) {
      return false;
    }
    const entry = Object.entries(this.journal.docs).find(
      ([, state]) => state.path === path
    );
    if (!entry) {
      return false;
    }
    const [docId] = entry;
    await this.client.forget({ docId, reason: "deleted in vault" });
    delete this.journal.docs[docId];
    await this.save();
    return true;
  }

  /**
   * True when write-back is pointed at the commons ("default") tenant and the user
   * has not opted in. Surfaces a single Notice the first time a push is blocked.
   */
  private isCommonsBlocked(): boolean {
    if (this.settings.allowCommonsWriteback) {
      return false;
    }
    if (!isCommonsTenant(this.settings.tenant)) {
      return false;
    }
    if (!this.commonsWarningShown) {
      this.commonsWarningShown = true;
      new Notice(
        'Theorem: write-back is blocked because the tenant is the commons ("default"). ' +
          'Set a personal tenant, or enable "Allow commons write-back" in settings.'
      );
    }
    return true;
  }

  private buildArgs(
    file: TFile,
    frontmatter: Record<string, unknown> | null,
    body: string,
    existingDocId: string | undefined
  ): UpsertNoteArgs {
    const title = asString(frontmatter?.title) || deriveTitle(file);
    const kind = (asString(frontmatter?.kind) || this.settings.defaultKind).toLowerCase();
    const args: UpsertNoteArgs = {
      tenant: this.settings.tenant,
      doc_id: existingDocId,
      title,
      content: body,
      kind,
      summary: asString(frontmatter?.summary) ?? "",
      tags: normalizeTags(frontmatter?.tags),
      links: this.resolveLinks(file, body),
      memory_node_type: asString(frontmatter?.memory_node_type) ?? "",
    };
    if (ENCODE_KINDS.has(kind)) {
      args.outcome = asString(frontmatter?.outcome) ?? "neutral";
      args.signal = asString(frontmatter?.signal) ?? "";
      args.reason = asString(frontmatter?.reason) ?? "";
      args.event_id = asString(frontmatter?.event_id) ?? "";
    }
    return args;
  }

  /**
   * Resolve each wikilink to the target note's doc_id when it exists, otherwise pass
   * the link text as a forward reference. The harness records unresolved targets and
   * reconciles them when the target note is later created.
   */
  private resolveLinks(file: TFile, body: string): string[] {
    const links = new Set<string>();
    for (const target of extractWikilinks(body)) {
      const dest = this.app.metadataCache.getFirstLinkpathDest(target, file.path);
      if (dest instanceof TFile) {
        const destFm = this.app.metadataCache.getFileCache(dest)?.frontmatter as
          | Record<string, unknown>
          | undefined;
        const destDocId = asString(destFm?.doc_id);
        links.add(destDocId ?? target);
      } else {
        links.add(target);
      }
    }
    return [...links];
  }

  private async writeDocIdBack(file: TFile, docId: string): Promise<void> {
    await this.guard.write(file.path, () =>
      this.app.fileManager.processFrontMatter(file, (fm) => {
        fm.doc_id = docId;
        if (fm.source === undefined) {
          fm.source = "theorem-harness";
        }
      })
    );
  }
}

function asString(value: unknown): string | undefined {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return undefined;
}

function normalizeTags(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value.map((item) => String(item).trim()).filter(Boolean);
  }
  if (typeof value === "string") {
    return value
      .split(/[,\s]+/)
      .map((item) => item.trim())
      .filter(Boolean);
  }
  return [];
}

/** Derive a title from the filename, stripping the trailing `-<shortid>` if present. */
function deriveTitle(file: TFile): string {
  const base = file.basename;
  const match = base.match(/^(.*)-([a-zA-Z0-9]{1,8})$/);
  if (match && match[2].length >= 4) {
    return humanize(match[1]);
  }
  return humanize(base);
}

function humanize(slug: string): string {
  const text = slug.replace(/[-_]+/g, " ").trim();
  return text ? text.charAt(0).toUpperCase() + text.slice(1) : slug;
}
