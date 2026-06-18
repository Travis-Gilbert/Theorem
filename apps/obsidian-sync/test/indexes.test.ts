// Generated Map-of-Content indexes: per-kind annotated lists + the root map.

import { test } from "node:test";
import assert from "node:assert/strict";
import { generateIndexFiles, type IndexedNote } from "../src/indexes";
import type { HarnessSyncSettings } from "../src/settings";

function settings(over: Partial<HarnessSyncSettings> = {}): HarnessSyncSettings {
  return {
    baseUrl: "",
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
    folderByKind: true,
    generateIndexes: true,
    indexFileName: "📍 Memory Map",
    conflictMode: "conflict-copy",
    defaultKind: "note",
    ...over,
  };
}

const NOTES: IndexedNote[] = [
  { basename: "fix-the-thing", title: "Fix the thing", kind: "solution", summary: "How it was fixed." },
  { basename: "another-fix", title: "Another fix", kind: "solution", summary: "" },
  { basename: "why-we-chose-x", title: "Why we chose X", kind: "decision", summary: "Rationale." },
];

test("generateIndexFiles returns nothing when disabled", () => {
  assert.deepEqual(generateIndexFiles(NOTES, settings({ generateIndexes: false })), []);
});

test("per-kind index lists notes with counts and summaries, carries the generated flag", () => {
  const files = generateIndexFiles(NOTES, settings());
  const solutions = files.find((f) => f.path === "Theorem/Solutions/_Solutions.md");
  assert.ok(solutions, "a Solutions index was generated");
  assert.ok(solutions!.content.includes("theorem_generated: index"), "carries the generated flag");
  assert.ok(solutions!.content.includes("# Solutions"));
  assert.ok(solutions!.content.includes("2 notes."), "count reflects the two solutions");
  assert.ok(solutions!.content.includes("[[another-fix|Another fix]]"));
  assert.ok(solutions!.content.includes("[[fix-the-thing|Fix the thing]]"));
  assert.ok(solutions!.content.includes("How it was fixed."), "summary is annotated");

  const decisions = files.find((f) => f.path === "Theorem/Decisions/_Decisions.md");
  assert.ok(decisions, "a Decisions index was generated");
  assert.ok(decisions!.content.includes("1 note."), "singular noun for one note");
});

test("root map links the sections with counts and embeds Dataview blocks", () => {
  const files = generateIndexFiles(NOTES, settings());
  const root = files.find((f) => f.path === "Theorem/📍 Memory Map.md");
  assert.ok(root, "the root map was generated at the configured name");
  const c = root!.content;
  assert.ok(c.includes("theorem_generated: index"));
  assert.ok(c.includes("# 📍 Memory Map"));
  assert.ok(c.includes("[[_Solutions|Solutions]] (2)"), "section link with count");
  assert.ok(c.includes("[[_Decisions|Decisions]] (1)"));
  assert.ok(c.includes("```dataview"), "has Dataview blocks for the Dataview users");
  assert.ok(c.includes('from "Theorem"'));
  assert.ok(c.includes('where theorem_generated != "index"'), "Dataview excludes the index notes");
});

test("flat layout (folderByKind off) emits only the root map", () => {
  const files = generateIndexFiles(NOTES, settings({ folderByKind: false }));
  assert.equal(files.length, 1);
  assert.equal(files[0].path, "Theorem/📍 Memory Map.md");
  assert.ok(files[0].content.includes("folder-by-kind is off"));
});

test("counts in the root reflect the total note count", () => {
  const files = generateIndexFiles(NOTES, settings());
  const root = files.find((f) => f.path.endsWith("Memory Map.md"))!;
  assert.ok(root.content.includes("(3 notes)"), "root states the total");
});
