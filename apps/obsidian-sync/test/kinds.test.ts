// Pull-time kind filter: the pure logic that keeps orchestrate exhaust out of
// the vault without burying real memory docs.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  normalizeKind,
  parseKindList,
  formatKindList,
  kindAllowed,
  filterDocsByKind,
} from "../src/kinds";
import type { HarnessDoc } from "../src/types";

function doc(kind: string, id = kind): HarnessDoc {
  return {
    doc_id: `doc_${id}`,
    kind,
    title: id,
    summary: "",
    content: "x",
    content_hash: "h",
    status: "active",
    tags: [],
    links: [],
    created_at: "",
    updated_at: "",
  };
}

test("default excludeKinds drops orchestrate, keeps real memory kinds", () => {
  const docs = [doc("orchestrate", "a"), doc("solution", "b"), doc("feedback", "c")];
  const kept = filterDocsByKind(docs, { includeKinds: [], excludeKinds: ["orchestrate"] });
  assert.deepEqual(
    kept.map((d) => d.kind),
    ["solution", "feedback"]
  );
});

test("empty filters keep everything", () => {
  const docs = [doc("orchestrate"), doc("note")];
  assert.equal(filterDocsByKind(docs, { includeKinds: [], excludeKinds: [] }).length, 2);
});

test("includeKinds is an allowlist: only listed kinds pass", () => {
  const docs = [doc("orchestrate"), doc("solution"), doc("note")];
  const kept = filterDocsByKind(docs, { includeKinds: ["solution"], excludeKinds: [] });
  assert.deepEqual(
    kept.map((d) => d.kind),
    ["solution"]
  );
});

test("exclude wins over include when a kind is in both lists", () => {
  const docs = [doc("solution")];
  const kept = filterDocsByKind(docs, { includeKinds: ["solution"], excludeKinds: ["solution"] });
  assert.equal(kept.length, 0);
});

test("kind matching is case-insensitive and whitespace-trimmed", () => {
  assert.equal(kindAllowed("  Orchestrate ", [], ["orchestrate"]), false);
  assert.equal(kindAllowed("SOLUTION", ["solution"], []), true);
  assert.equal(normalizeKind("  Feedback  "), "feedback");
});

test("undefined filter arrays are treated as empty (older persisted settings)", () => {
  const docs = [doc("orchestrate")];
  const kept = filterDocsByKind(docs, {
    includeKinds: undefined as unknown as string[],
    excludeKinds: undefined as unknown as string[],
  });
  assert.equal(kept.length, 1);
});

test("parseKindList normalizes, splits on commas/spaces, and de-dups", () => {
  assert.deepEqual(parseKindList("Orchestrate, solution  solution,, FEEDBACK"), [
    "orchestrate",
    "solution",
    "feedback",
  ]);
  assert.deepEqual(parseKindList("   "), []);
});

test("formatKindList round-trips a list for the settings UI", () => {
  assert.equal(formatKindList(["orchestrate", "note"]), "orchestrate, note");
  assert.equal(formatKindList([]), "");
  assert.equal(formatKindList(undefined), "");
});
