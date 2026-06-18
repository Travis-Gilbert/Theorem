// Navigable-vault emission, end to end through the real Syncer:
//  - graph-internal kinds (community_summary, orchestrate) are not written
//  - real notes land in per-kind folders with human filenames
//  - Map-of-Content indexes are generated
//  - a title change renames the note in place (no duplicate)
//  - a kind change moves the note to the new kind folder (no duplicate)

import { test } from "node:test";
import assert from "node:assert/strict";
import { TFile, TFolder, type App } from "obsidian";
import { Syncer } from "../src/sync";
import { SyncGuard } from "../src/guard";
import { emptyJournal } from "../src/types";
import type { HarnessDoc, ListDocsResponse, SyncJournal } from "../src/types";
import type { HarnessClient } from "../src/harness";
import type { HarnessSyncSettings } from "../src/settings";

class FakeVault {
  files = new Map<string, string>();
  folders = new Set<string>();
  getAbstractFileByPath(path: string): TFile | TFolder | null {
    if (this.files.has(path)) return new TFile(path);
    if (this.folders.has(path)) return new TFolder(path);
    return null;
  }
  async read(file: TFile): Promise<string> {
    const c = this.files.get(file.path);
    if (c === undefined) throw new Error(`read: no such file ${file.path}`);
    return c;
  }
  async create(path: string, content: string): Promise<TFile> {
    this.files.set(path, content);
    return new TFile(path);
  }
  async modify(file: TFile, content: string): Promise<void> {
    this.files.set(file.path, content);
  }
  async createFolder(path: string): Promise<void> {
    this.folders.add(path);
  }
  async renameFile(file: TFile, target: string): Promise<void> {
    const content = this.files.get(file.path) ?? "";
    this.files.delete(file.path);
    this.files.set(target, content);
  }
}

class FakeApp {
  vault = new FakeVault();
  fileManager = {
    renameFile: (file: TFile, target: string) => this.vault.renameFile(file, target),
  };
}

/** A client whose doc set can be mutated between pulls. */
class MutableClient {
  constructor(public docs: HarnessDoc[], public maxUpdatedAt = "unix_ms:1000") {}
  async listDocs(_since: string): Promise<ListDocsResponse> {
    return { ok: true, tenant: "tester", count: this.docs.length, max_updated_at: this.maxUpdatedAt, docs: this.docs };
  }
}

function doc(
  doc_id: string,
  kind: string,
  title: string,
  content_hash: string,
  updated_at: string
): HarnessDoc {
  return {
    doc_id,
    kind,
    title,
    summary: `summary of ${title}`,
    content: `body of ${title}`,
    content_hash,
    status: "active",
    tags: [],
    links: [],
    created_at: "unix_ms:1000",
    updated_at,
  };
}

const settings: HarnessSyncSettings = {
  baseUrl: "https://example.test",
  token: "",
  tenant: "tester",
  syncFolder: "Theorem",
  captureFolder: "",
  captureFlag: "graph",
  enableWriteBack: false,
  allowCommonsWriteback: false,
  syncIntervalMinutes: 0,
  includeInactive: false,
  includeKinds: [],
  excludeKinds: ["community_summary", "orchestrate"],
  folderByKind: true,
  generateIndexes: true,
  indexFileName: "📍 Memory Map",
  conflictMode: "conflict-copy",
  defaultKind: "note",
};

function makeSyncer(client: MutableClient, journal: SyncJournal, app: FakeApp): Syncer {
  return new Syncer(
    app as unknown as App,
    client as unknown as HarnessClient,
    settings,
    journal,
    new SyncGuard(),
    async () => {}
  );
}

test("a mixed batch yields a navigable, filtered, foldered vault with indexes", async () => {
  const client = new MutableClient([
    doc("doc_sol1", "solution", "Fix The Bug", "h1", "unix_ms:1000"),
    doc("doc_dec1", "decision", "Choose Postgres", "h1", "unix_ms:1000"),
    doc("doc_cs1", "community_summary", "Community 0", "h1", "unix_ms:1000"),
    doc("doc_orc1", "orchestrate", "Coordination Log", "h1", "unix_ms:1000"),
  ]);
  const app = new FakeApp();
  const summary = await makeSyncer(client, emptyJournal(), app).pull();

  assert.equal(summary.created, 2, "only the two real notes are written");
  assert.equal(summary.filtered, 2, "community_summary + orchestrate are filtered");

  const paths = [...app.vault.files.keys()];
  // Real notes land in kind folders with human filenames (no doc-id suffix).
  assert.ok(paths.includes("Theorem/Solutions/fix-the-bug.md"), paths.join(", "));
  assert.ok(paths.includes("Theorem/Decisions/choose-postgres.md"), paths.join(", "));
  // No note for an excluded kind.
  assert.ok(!paths.some((p) => p.includes("community-0")), "no community_summary note");
  assert.ok(!paths.some((p) => p.includes("coordination-log")), "no orchestrate note");
  // Indexes generated.
  assert.ok(paths.includes("Theorem/Solutions/_Solutions.md"), "per-kind Solutions index");
  assert.ok(paths.includes("Theorem/Decisions/_Decisions.md"), "per-kind Decisions index");
  assert.ok(paths.includes("Theorem/📍 Memory Map.md"), "root Memory Map");

  // The root map links the sections; the solution note carries doc identity.
  const root = app.vault.files.get("Theorem/📍 Memory Map.md")!;
  assert.ok(root.includes("[[_Solutions|Solutions]] (1)"));
  const note = app.vault.files.get("Theorem/Solutions/fix-the-bug.md")!;
  assert.ok(note.includes("doc_id: doc_sol1"), "identity is in frontmatter, not the filename");
});

test("a title change renames the note in place, without duplicating it", async () => {
  const client = new MutableClient([doc("doc_sol1", "solution", "Fix The Bug", "h1", "unix_ms:1000")]);
  const app = new FakeApp();
  const journal = emptyJournal();
  await makeSyncer(client, journal, app).pull();
  assert.ok(app.vault.files.has("Theorem/Solutions/fix-the-bug.md"));

  // Re-pull the same doc_id with a new title and bumped hash/watermark.
  client.docs = [doc("doc_sol1", "solution", "Fix The Crash", "h2", "unix_ms:2000")];
  client.maxUpdatedAt = "unix_ms:2000";
  await makeSyncer(client, journal, app).pull();

  assert.ok(!app.vault.files.has("Theorem/Solutions/fix-the-bug.md"), "old note removed");
  assert.ok(app.vault.files.has("Theorem/Solutions/fix-the-crash.md"), "renamed to new title");
  const solutionNotes = [...app.vault.files.keys()].filter(
    (p) => p.startsWith("Theorem/Solutions/") && !p.includes("_Solutions")
  );
  assert.equal(solutionNotes.length, 1, "exactly one note for the doc, no duplicate");
});

test("a kind change moves the note to the new kind folder", async () => {
  const client = new MutableClient([doc("doc_sol1", "solution", "Movable Note", "h1", "unix_ms:1000")]);
  const app = new FakeApp();
  const journal = emptyJournal();
  await makeSyncer(client, journal, app).pull();
  assert.ok(app.vault.files.has("Theorem/Solutions/movable-note.md"));

  client.docs = [doc("doc_sol1", "decision", "Movable Note", "h2", "unix_ms:2000")];
  client.maxUpdatedAt = "unix_ms:2000";
  await makeSyncer(client, journal, app).pull();

  assert.ok(!app.vault.files.has("Theorem/Solutions/movable-note.md"), "left the Solutions folder");
  assert.ok(app.vault.files.has("Theorem/Decisions/movable-note.md"), "moved into Decisions");
});
