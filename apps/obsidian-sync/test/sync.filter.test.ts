// Pull-time kind filter, end to end through the real Syncer.
//
// A mixed batch (orchestrate exhaust + one real solution) must write a note
// only for the kept kind, report the dropped count as `filtered`, and never
// create a file for an excluded kind. This is the behavior that unblocks a
// clean first sync against a tenant dominated by orchestrate docs.

import { test } from "node:test";
import assert from "node:assert/strict";
import { TFile, TFolder, type App } from "obsidian";
import { Syncer } from "../src/sync";
import { SyncGuard } from "../src/guard";
import { emptyJournal } from "../src/types";
import type { HarnessDoc, ListDocsResponse } from "../src/types";
import type { HarnessClient } from "../src/harness";
import type { HarnessSyncSettings } from "../src/settings";

// --- in-memory vault (minimal, matching sync.pull.test) -------------------

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

class BatchClient {
  calls = 0;
  constructor(private docs: HarnessDoc[]) {}
  async listDocs(_since: string): Promise<ListDocsResponse> {
    this.calls += 1;
    return {
      ok: true,
      tenant: "tester",
      count: this.docs.length,
      max_updated_at: "unix_ms:2000",
      docs: this.docs,
    };
  }
}

function makeDoc(id: string, kind: string, title: string): HarnessDoc {
  return {
    doc_id: id,
    kind,
    title,
    summary: "",
    content: `body of ${title}`,
    content_hash: `hash-${id}`,
    status: "active",
    tags: [],
    links: [],
    created_at: "unix_ms:1000",
    updated_at: "unix_ms:1000",
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
  excludeKinds: ["orchestrate"],
  conflictMode: "conflict-copy",
  defaultKind: "note",
};

test("pull drops excluded kinds: only real-memory notes are written", async () => {
  const docs = [
    makeDoc("doc_o1", "orchestrate", "Coordination One"),
    makeDoc("doc_o2", "orchestrate", "Coordination Two"),
    makeDoc("doc_s1", "solution", "Real Solution"),
  ];
  const app = new FakeApp();
  const syncer = new Syncer(
    app as unknown as App,
    new BatchClient(docs) as unknown as HarnessClient,
    settings,
    emptyJournal(),
    new SyncGuard(),
    async () => {}
  );

  const summary = await syncer.pull();

  assert.equal(summary.pulled, 1, "one doc passed the filter");
  assert.equal(summary.filtered, 2, "two orchestrate docs were dropped");
  assert.equal(summary.created, 1, "the solution note was created");

  const paths = [...app.vault.files.keys()];
  assert.equal(paths.length, 1, "no note exists for an excluded kind");
  assert.ok(
    paths[0].includes("real-solution"),
    `expected the solution note, got ${paths[0]}`
  );
});

test("empty excludeKinds mirrors the whole batch", async () => {
  const docs = [
    makeDoc("doc_o1", "orchestrate", "Coordination One"),
    makeDoc("doc_s1", "solution", "Real Solution"),
  ];
  const app = new FakeApp();
  const syncer = new Syncer(
    app as unknown as App,
    new BatchClient(docs) as unknown as HarnessClient,
    { ...settings, excludeKinds: [] },
    emptyJournal(),
    new SyncGuard(),
    async () => {}
  );

  const summary = await syncer.pull();

  assert.equal(summary.filtered, 0, "nothing filtered with an empty denylist");
  assert.equal(summary.created, 2, "both docs become notes");
  assert.equal(app.vault.files.size, 2);
});
