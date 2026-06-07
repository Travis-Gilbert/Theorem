import { App, TFile, TFolder, normalizePath } from "obsidian";
import type { HarnessClient } from "./harness";
import type { HarnessSyncSettings } from "./settings";
import type { HarnessDoc, SyncJournal } from "./types";
import { localHash } from "./hash";
import {
  LinkResolver,
  noteBasename,
  notePath,
  renderNote,
  userBody,
} from "./notes";
import type { SyncGuard } from "./guard";

export interface SyncSummary {
  pulled: number;
  created: number;
  updated: number;
  skipped: number;
  conflicts: number;
}

/**
 * The pull half of the sync. Mirrors the tenant's memory docs into the vault and
 * enforces the echo guards: the hash gate (skip docs whose content matches what we
 * last wrote), remote-write suppression (so pull writes never echo back), and
 * conflict surfacing (write a conflict copy when both sides changed).
 */
export class Syncer {
  constructor(
    private app: App,
    private client: HarnessClient,
    private settings: HarnessSyncSettings,
    private journal: SyncJournal,
    private guard: SyncGuard,
    private save: () => Promise<void>
  ) {}

  async pull(): Promise<SyncSummary> {
    const response = await this.client.listDocs(this.journal.watermark);
    const summary: SyncSummary = {
      pulled: response.docs.length,
      created: 0,
      updated: 0,
      skipped: 0,
      conflicts: 0,
    };

    const byId = new Map(response.docs.map((doc) => [doc.doc_id, doc]));
    const resolveLink: LinkResolver = (target) => {
      const inBatch = byId.get(target);
      if (inBatch) {
        return { basename: noteBasename(inBatch.title, inBatch.doc_id), title: inBatch.title };
      }
      const known = this.journal.docs[target];
      if (known) {
        return { basename: basenameOf(known.path), title: known.title };
      }
      return null;
    };

    await this.ensureFolder(this.settings.syncFolder);

    this.guard.beginRemote();
    try {
      for (const doc of response.docs) {
        const outcome = await this.applyDoc(doc, resolveLink);
        summary[outcome] += 1;
      }
    } finally {
      this.guard.endRemote();
    }

    if (response.max_updated_at && response.max_updated_at > this.journal.watermark) {
      this.journal.watermark = response.max_updated_at;
    }
    await this.save();
    return summary;
  }

  private async applyDoc(
    doc: HarnessDoc,
    resolveLink: LinkResolver
  ): Promise<"created" | "updated" | "skipped" | "conflicts"> {
    const folder = this.settings.syncFolder;
    const targetPath = normalizePath(notePath(folder, noteBasename(doc.title, doc.doc_id)));
    const desired = renderNote(doc, resolveLink);
    const desiredBody = (doc.content ?? "").trim();
    const state = this.journal.docs[doc.doc_id];

    const current = this.resolveCurrentFile(state?.path, targetPath);

    if (!current) {
      await this.writeNew(targetPath, desired);
      this.record(doc, targetPath, desiredBody);
      return "created";
    }

    const currentText = await this.app.vault.read(current);
    const currentBody = userBody(currentText);
    const graphChanged = !state || state.contentHash !== doc.content_hash;
    const localEdited = !state || localHash(currentBody) !== state.bodyHash;

    if (!graphChanged && !localEdited) {
      // Nothing changed; keep bookkeeping current (e.g. after a rename) and move on.
      await this.placeAtTarget(current, targetPath);
      this.record(doc, this.pathOf(current, targetPath), desiredBody);
      return "skipped";
    }

    if (graphChanged && localEdited && currentBody !== desiredBody) {
      return this.resolveConflict(doc, current, desired);
    }

    if (!graphChanged && localEdited) {
      // The user edited locally but the graph did not move. This is a write-back
      // candidate, not a pull action; leave the file for the write-back path.
      return "skipped";
    }

    // Graph changed (and either local matches the new graph content, or local was
    // not independently edited): bring the note in line with the graph.
    const placed = await this.placeAtTarget(current, targetPath);
    if (currentText !== desired) {
      await this.guard.write(this.pathOf(placed, targetPath), () =>
        this.app.vault.modify(placed, desired)
      );
    }
    this.record(doc, this.pathOf(placed, targetPath), desiredBody);
    return "updated";
  }

  private async resolveConflict(
    doc: HarnessDoc,
    current: TFile,
    desired: string
  ): Promise<"updated" | "skipped" | "conflicts"> {
    if (this.settings.conflictMode === "graph-wins") {
      await this.guard.write(current.path, () => this.app.vault.modify(current, desired));
      this.record(doc, current.path, (doc.content ?? "").trim());
      return "updated";
    }
    if (this.settings.conflictMode === "local-wins") {
      // Keep local; record the incoming hash so we do not re-flag the same conflict.
      this.touchContentHash(doc);
      return "skipped";
    }
    // Default: write the incoming graph version beside the user's note.
    const conflictPath = conflictCopyPath(current.path);
    await this.writeNew(conflictPath, desired);
    this.touchContentHash(doc);
    return "conflicts";
  }

  // --- file helpers -------------------------------------------------------

  private resolveCurrentFile(statePath: string | undefined, targetPath: string): TFile | null {
    if (statePath) {
      const atState = this.app.vault.getAbstractFileByPath(statePath);
      if (atState instanceof TFile) {
        return atState;
      }
    }
    const atTarget = this.app.vault.getAbstractFileByPath(targetPath);
    return atTarget instanceof TFile ? atTarget : null;
  }

  /** Rename a file to its target path when the title-derived basename changed. */
  private async placeAtTarget(file: TFile, targetPath: string): Promise<TFile> {
    if (file.path === targetPath) {
      return file;
    }
    if (this.app.vault.getAbstractFileByPath(targetPath)) {
      return file; // target name taken; keep current path
    }
    await this.guard.write(targetPath, async () => {
      await this.guard.write(file.path, () => this.app.fileManager.renameFile(file, targetPath));
    });
    const renamed = this.app.vault.getAbstractFileByPath(targetPath);
    return renamed instanceof TFile ? renamed : file;
  }

  private pathOf(file: TFile, fallback: string): string {
    return file?.path ?? fallback;
  }

  private async writeNew(path: string, content: string): Promise<void> {
    await this.guard.write(path, async () => {
      const existing = this.app.vault.getAbstractFileByPath(path);
      if (existing instanceof TFile) {
        await this.app.vault.modify(existing, content);
      } else {
        await this.app.vault.create(path, content);
      }
    });
  }

  private record(doc: HarnessDoc, path: string, body: string): void {
    this.journal.docs[doc.doc_id] = {
      contentHash: doc.content_hash,
      bodyHash: localHash(body),
      path,
      title: doc.title,
      updatedAt: doc.updated_at,
    };
  }

  private touchContentHash(doc: HarnessDoc): void {
    const state = this.journal.docs[doc.doc_id];
    if (state) {
      state.contentHash = doc.content_hash;
    } else {
      this.journal.docs[doc.doc_id] = {
        contentHash: doc.content_hash,
        bodyHash: "",
        path: "",
        title: doc.title,
        updatedAt: doc.updated_at,
      };
    }
  }

  private async ensureFolder(folder: string): Promise<void> {
    if (!folder) {
      return;
    }
    const segments = folder.split("/").filter(Boolean);
    let path = "";
    for (const segment of segments) {
      path = path ? `${path}/${segment}` : segment;
      const existing = this.app.vault.getAbstractFileByPath(path);
      if (!existing) {
        await this.app.vault.createFolder(path).catch(() => undefined);
      } else if (existing instanceof TFile) {
        throw new Error(`Sync folder path "${path}" is occupied by a file.`);
      } else if (!(existing instanceof TFolder)) {
        throw new Error(`Sync folder path "${path}" is not a folder.`);
      }
    }
  }
}

function basenameOf(path: string): string {
  const file = path.split("/").pop() ?? path;
  return file.replace(/\.md$/, "");
}

function conflictCopyPath(path: string): string {
  return path.replace(/\.md$/, "") + " (graph conflict).md";
}
