// Navigable-vault layout: human basenames with collision handling, kind folders,
// and the computed "Related" block.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  assignBasenames,
  kindFolder,
  renderRelatedBlock,
  type LinkResolver,
} from "../src/notes";

test("assignBasenames uses the human title slug, no doc-id suffix", () => {
  const map = assignBasenames([
    { doc_id: "doc_abc123", title: "A Real Solution" },
    { doc_id: "doc_def456", title: "Some Decision" },
  ]);
  assert.equal(map.get("doc_abc123"), "a-real-solution");
  assert.equal(map.get("doc_def456"), "some-decision");
});

test("assignBasenames disambiguates a title collision with a short doc-id", () => {
  const map = assignBasenames([
    { doc_id: "doc_first0001", title: "Same Title" },
    { doc_id: "doc_second002", title: "Same Title" },
  ]);
  assert.equal(map.get("doc_first0001"), "same-title", "first claimer keeps the clean name");
  const second = map.get("doc_second002")!;
  assert.notEqual(second, "same-title", "the collision is disambiguated");
  assert.ok(second.startsWith("same-title-"), `got ${second}`);
});

test("assignBasenames seeds reserved names so incremental pulls do not collide", () => {
  // "foo" is already on disk owned by another doc; a new doc titled Foo must not reuse it.
  const map = assignBasenames(
    [{ doc_id: "doc_new", title: "Foo" }],
    [["foo", "doc_existing"]]
  );
  const assigned = map.get("doc_new")!;
  assert.notEqual(assigned, "foo");
  assert.ok(assigned.startsWith("foo-"), `got ${assigned}`);
});

test("assignBasenames leaves a re-pulled doc on its own reserved name", () => {
  // Same doc_id owns "foo" in the reserved set: it should keep "foo", not disambiguate.
  const map = assignBasenames(
    [{ doc_id: "doc_same", title: "Foo" }],
    [["foo", "doc_same"]]
  );
  assert.equal(map.get("doc_same"), "foo");
});

test("assignBasenames falls back to 'untitled' for an empty title", () => {
  const map = assignBasenames([{ doc_id: "doc_x", title: "   " }]);
  assert.equal(map.get("doc_x"), "untitled");
});

test("kindFolder maps known kinds and falls back to Notes", () => {
  assert.equal(kindFolder("solution"), "Solutions");
  assert.equal(kindFolder("postmortem"), "Postmortems");
  assert.equal(kindFolder("decision"), "Decisions");
  assert.equal(kindFolder("feedback"), "Feedback");
  assert.equal(kindFolder("self_revise"), "Revisions");
  assert.equal(kindFolder("encode"), "Notes");
  assert.equal(kindFolder("note"), "Notes");
  assert.equal(kindFolder("something_unmapped"), "Notes");
  assert.equal(kindFolder(undefined), "Notes");
  assert.equal(kindFolder("  SOLUTION "), "Solutions", "case/space-insensitive");
});

test("renderRelatedBlock emits a delimited Related block, or nothing when empty", () => {
  const resolve: LinkResolver = (target) =>
    target === "doc_n1" ? { basename: "neighbor-one", title: "Neighbor One" } : null;

  assert.equal(renderRelatedBlock([], resolve), "");

  const block = renderRelatedBlock(["doc_n1", "doc_missing"], resolve);
  assert.ok(block.includes("%% theorem:related:start %%"));
  assert.ok(block.includes("%% theorem:related:end %%"));
  assert.ok(block.includes("## Related"));
  assert.ok(block.includes("[[neighbor-one|Neighbor One]]"), "resolved neighbor renders with alias");
  assert.ok(block.includes("[[doc_missing]]"), "unresolved neighbor renders as a raw wikilink");
});
