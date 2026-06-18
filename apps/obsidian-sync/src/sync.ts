import { App, TFile, TFolder, normalizePath } from "obsidian";
import type { HarnessClient } from "./harness";
import type { HarnessSyncSettings } from "./settings";
import type { HarnessDoc, SyncJournal } from "./types";
import { localHash } from "./hash";
import { filterDocsByKind } from "./kinds";
import {
  assignBasenames,
  kindFolder,
  LinkResolver,
  renderNote,
  userBody,
} from "./notes";
import { generateIndexFiles, type IndexedNote } from "./indexes";
import type { SyncGuard } from "./guard";

export interface SyncSummary {
  /** Docs that passed the kind filter and were processed this pull. */
  pulled: number;
  /** Docs the server returned that the kind filter dropped before processing. */
  filtered: number;
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
    // Drop excluded kinds (e.g. orchestrate exhaust) before writing any notes.
    // The filter is a pull policy: it decides which graph docs become notes and
    // never touches write-back.
    const kept = filterDocsByKind(response.docs, this.settings);
    const summary: SyncSummary = {
      pulled: kept.length,
      filtered: response.docs.length - kept.length,
      created: 0,
      updated: 0,
      skipped: 0,
      conflicts: 0,
    };

    // Assign each kept doc a human, globally-unique basename from its title.
    // Identity stays in the frontmatter doc_id, so filenames can be clean and
    // change on rename. Seed the reservation set from the journal so an
    // incremental pull (only changed docs) does not collide with notes on disk.
    const reserved: Array<[string, string]> = Object.entries(this.journal.docs)
      .filter(([, state]) => state.path)
      .map(([docId, state]) => [basenameOf(state.path), docId]);
    const basenames = assignBasenames(kept, reserved);

    // Resolve a link target to its note basename. Prefer the freshly-assigned
    // basenames (so wikilinks match the files written this pull), then the journal
    // for targets outside this batch (including filtered docs already on disk).
    const byId = new Map(response.docs.map((doc) => [doc.doc_id, doc]));
    const resolveLink: LinkResolver = (target) => {
      const assigned = basenames.get(target);
      if (assigned) {
        const doc = byId.get(target);
        return { basename: assigned, title: doc?.title ?? assigned };
      }
      const known = this.journal.docs[target];
      if (known) {
        return { basename: basenameOf(known.path), title: known.title };
      }
      return null;
    };

    await this.ensureFolder(this.settings.syncFolder);
    if (this.settings.folderByKind) {
      const folders = new Set(kept.map((doc) => kindFolder(doc.kind)));
      for (const folder of folders) {
        await this.ensureFolder(`${this.settings.syncFolder}/${folder}`);
      }
    }

    this.guard.beginRemote();
    try {
      for (const doc of kept) {
        const basename = basenames.get(doc.doc_id) ?? "untitled";
        const targetPath = normalizePath(
          notePathFor(this.settings.syncFolder, doc.kind, basename, this.settings.folderByKind)
        );
        const outcome = await this.applyDoc(doc, targetPath, resolveLink);
        summary[outcome] += 1;
      }
    } finally {
      this.guard.endRemote();
    }

    await this.writeIndexes();

    if (response.max_updated_at && response.max_updated_at > this.journal.watermark) {
      this.journal.watermark = response.max_updated_at;
    }
    await this.save();
    return summary;
  }

  private async applyDoc(
    doc: HarnessDoc,
    targetPath: string,
    resolveLink: LinkResolver
  ): Promise<"created" | "updated" | "skipped" | "conflicts"> {
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
      kind: doc.kind,
      summary: doc.summary,
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
        kind: doc.kind,
        summary: doc.summary,
      };
    }
  }

  /**
   * Regenerate the Map-of-Content indexes from the journal (every synced note, not
   * just this batch). Plugin-owned: each carries the generated flag, so write-back
   * skips them; the read-compare avoids rewriting an unchanged index.
   */
  private async writeIndexes(): Promise<void> {
    if (!this.settings.generateIndexes) {
      return;
    }
    const notes: IndexedNote[] = Object.values(this.journal.docs)
      .filter((state) => state.path)
      .map((state) => ({
        basename: basenameOf(state.path),
        title: state.title,
        kind: state.kind ?? "",
        summary: state.summary ?? "",
      }));
    for (const file of generateIndexFiles(notes, this.settings)) {
      await this.writeIndexFile(normalizePath(file.path), file.content);
    }
  }

  private async writeIndexFile(path: string, content: string): Promise<void> {
    const parent = path.split("/").slice(0, -1).join("/");
    await this.ensureFolder(parent);
    await this.guard.write(path, async () => {
      const existing = this.app.vault.getAbstractFileByPath(path);
      if (existing instanceof TFile) {
        const current = await this.app.vault.read(existing);
        if (current !== content) {
          await this.app.vault.modify(existing, content);
        }
      } else {
        await this.app.vault.create(path, content);
      }
    });
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

function notePathFor(
  syncFolder: string,
  kind: string,
  basename: string,
  folderByKind: boolean
): string {
  const segments = [syncFolder];
  if (folderByKind) {
    segments.push(kindFolder(kind));
  }
  segments.push(`${basename}.md`);
  return segments.filter(Boolean).join("/");
}

function conflictCopyPath(path: string): string {
  return path.replace(/\.md$/, "") + " (graph conflict).md";
}
