// B3: steady-state pull writes nothing.
//
// The server `since` filter is boundary-inclusive: `since = max_updated_at`
// re-returns the single newest doc, which the hash gate must then skip rather
// than re-write. This test locks that in: a second consecutive pull() with no
// graph-side and no vault-side change reports only `skipped` — created,
// updated, and conflicts all zero. A regression that bumps the watermark past
// the boundary, or that stops skipping on a hash match, would surface here as
// an echo storm (every steady-state pull rewriting files).

import { test } from "node:test";
import assert from "node:assert/strict";
import { TFile, TFolder, type App } from "obsidian";
import { Syncer } from "../src/sync";
import { SyncGuard } from "../src/guard";
import { emptyJournal } from "../src/types";
import type { HarnessDoc, ListDocsResponse } from "../src/types";
import type { HarnessClient } from "../src/harness";
import type { HarnessSyncSettings } from "../src/settings";

// --- in-memory vault ------------------------------------------------------

class FakeVault {
  files = new Map<string, string>();
  folders = new Set<string>();

  getAbstractFileByPath(path: string): TFile | TFolder | null {
    if (this.files.has(path)) return new TFile(path);
    if (this.folders.has(path)) return new TFolder(path);
    return null;
  }
  async read(file: TFile): Promise<string> {
    const content = this.files.get(file.path);
    if (content === undefined) throw new Error(`read: no such file ${file.path}`);
    return content;
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

// --- fake harness client --------------------------------------------------

function makeDoc(): HarnessDoc {
  return {
    doc_id: "doc_alpha01",
    kind: "note",
    title: "Alpha One",
    summary: "",
    content: "The body of alpha.",
    content_hash: "hash-alpha-1",
    status: "active",
    tags: [],
    links: [],
    created_at: "unix_ms:1000",
    updated_at: "unix_ms:1000",
  };
}

class FakeClient {
  calls = 0;
  constructor(private doc: HarnessDoc) {}
  async listDocs(_since: string): Promise<ListDocsResponse> {
    this.calls += 1;
    // Boundary-inclusive server: the newest doc comes back again on every
    // pull, regardless of the watermark we send. That is the case under test.
    return {
      ok: true,
      tenant: "tester",
      count: 1,
      max_updated_at: this.doc.updated_at,
      docs: [this.doc],
    };
  }
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
  excludeKinds: [],
  folderByKind: false,
  generateIndexes: false,
  indexFileName: "📍 Memory Map",
  conflictMode: "conflict-copy",
  defaultKind: "note",
};

// --- the test -------------------------------------------------------------

test("steady-state pull writes nothing: second pull only skips", async () => {
  const doc = makeDoc();
  const app = new FakeApp();
  const client = new FakeClient(doc);
  const guard = new SyncGuard();
  const journal = emptyJournal();
  const save = async () => {};

  const syncer = new Syncer(
    app as unknown as App,
    client as unknown as HarnessClient,
    settings,
    journal,
    guard,
    save
  );

  const first = await syncer.pull();
  assert.equal(first.created, 1, "first pull creates the doc's note");
  assert.equal(first.updated, 0, "first pull updates nothing");
  assert.equal(first.conflicts, 0, "first pull has no conflicts");

  const second = await syncer.pull();
  assert.equal(second.created, 0, "second pull creates nothing");
  assert.equal(second.updated, 0, "second pull updates nothing");
  assert.equal(second.conflicts, 0, "second pull has no conflicts");
  assert.equal(second.skipped, second.pulled, "every pulled doc is skipped");
  assert.ok(second.skipped >= 1, "the re-returned newest doc is skipped");
  assert.equal(client.calls, 2, "each pull hit the list endpoint once");
});

test("steady-state pull leaves the vault file byte-identical", async () => {
  const doc = makeDoc();
  const app = new FakeApp();
  const client = new FakeClient(doc);
  const syncer = new Syncer(
    app as unknown as App,
    client as unknown as HarnessClient,
    settings,
    emptyJournal(),
    new SyncGuard(),
    async () => {}
  );

  await syncer.pull();
  const afterFirst = new Map(app.vault.files);
  await syncer.pull();

  assert.deepEqual(
    [...app.vault.files.entries()].sort(),
    [...afterFirst.entries()].sort(),
    "a no-change pull must not rewrite any file"
  );
});
