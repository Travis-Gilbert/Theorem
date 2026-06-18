import type { HarnessSyncSettings } from "./settings";
import type { HarnessDoc } from "./types";

// The generated links block is delimited by Obsidian comment markers so it renders
// invisibly, yet the wikilinks between the markers stay clickable and show in the
// graph view. Write-back strips this block to recover the user's own body.
export const LINK_BLOCK_START = "%% theorem:links:start %%";
export const LINK_BLOCK_END = "%% theorem:links:end %%";

// The "Related" block holds computed semantic neighbors (MEMORY_SIMILAR edges),
// kept separate from the user's authored links so the two never conflate.
export const RELATED_BLOCK_START = "%% theorem:related:start %%";
export const RELATED_BLOCK_END = "%% theorem:related:end %%";

const FRONTMATTER_RE = /^---\n([\s\S]*?)\n---\n?/;
const WIKILINK_RE = /\[\[([^\]]+)\]\]/g;

/**
 * Kind -> folder map for the folder-by-kind layout. Unmapped kinds fall to "Notes".
 * Graph-internal kinds (community_summary, orchestrate) are filtered before this
 * is reached, so they need no entry.
 */
const KIND_FOLDER: Record<string, string> = {
  postmortem: "Postmortems",
  solution: "Solutions",
  decision: "Decisions",
  feedback: "Feedback",
  encode: "Notes",
  note: "Notes",
  plan: "Notes",
  convention: "Notes",
  self_revise: "Revisions",
};

/** The subfolder a note of this kind lives in under the sync folder. */
export function kindFolder(kind: string | undefined): string {
  return KIND_FOLDER[(kind ?? "").trim().toLowerCase()] ?? "Notes";
}

/** Resolve a link target doc_id to the basename + title of its note, if known. */
export type LinkResolver = (target: string) => { basename: string; title: string } | null;

/**
 * Assign each doc a human, globally-unique note basename derived from its title.
 * Identity stays in the frontmatter `doc_id`, so the filename can be clean and
 * mutable; collisions (two docs sharing a title slug) disambiguate with a short
 * doc_id fragment, then a counter. `reserved` seeds basenames already on disk
 * (from the journal) so an incremental pull does not collide with existing notes.
 * Returns a doc_id -> basename map.
 */
export function assignBasenames(
  docs: ReadonlyArray<{ doc_id: string; title: string }>,
  reserved?: Iterable<[string, string]>
): Map<string, string> {
  const owner = new Map<string, string>(); // basename -> first-claiming doc_id
  if (reserved) {
    for (const [base, docId] of reserved) {
      if (base) {
        owner.set(base, docId);
      }
    }
  }
  const out = new Map<string, string>(); // doc_id -> basename
  for (const doc of docs) {
    let base = slugify(doc.title) || "untitled";
    const held = owner.get(base);
    if (held !== undefined && held !== doc.doc_id) {
      const stem = `${base}-${shortDocId(doc.doc_id)}`;
      base = stem;
      let n = 2;
      while (owner.has(base) && owner.get(base) !== doc.doc_id) {
        base = `${stem}-${n}`;
        n += 1;
      }
    }
    owner.set(base, doc.doc_id);
    out.set(doc.doc_id, base);
  }
  return out;
}

/** The vault path for a note, honoring the folder-by-kind layout. */
export function notePathFor(
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

export function slugify(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80);
}

export function shortDocId(docId: string): string {
  const alnum = docId.replace(/[^a-zA-Z0-9]/g, "");
  return alnum.slice(-8) || "0";
}

/** Render a complete note file (frontmatter + body + generated links block). */
export function renderNote(doc: HarnessDoc, resolveLink: LinkResolver): string {
  const frontmatter = buildFrontmatter({
    doc_id: doc.doc_id,
    kind: doc.kind,
    title: doc.title,
    summary: doc.summary,
    status: doc.status,
    tags: doc.tags ?? [],
    content_hash: doc.content_hash,
    created_at: doc.created_at,
    updated_at: doc.updated_at,
    source: "theorem-harness",
  });

  const body = doc.content ?? "";
  const links = renderLinksBlock(doc.links ?? [], resolveLink);
  const related = renderRelatedBlock((doc.similar ?? []).map((s) => s.doc_id), resolveLink);
  return `${frontmatter}\n${body}${links}${related}`;
}

/** Render a delimited managed wikilink block, or "" when there are no targets. */
function renderManagedBlock(
  targets: string[],
  resolveLink: LinkResolver,
  startMarker: string,
  endMarker: string,
  heading: string
): string {
  if (!targets.length) {
    return "";
  }
  const lines = targets.map((target) => {
    const resolved = resolveLink(target);
    if (resolved) {
      const alias = resolved.title && resolved.title !== resolved.basename ? `|${resolved.title}` : "";
      return `- [[${resolved.basename}${alias}]]`;
    }
    // Unresolved forward reference: render the raw target as a dangling wikilink.
    return `- [[${target}]]`;
  });
  return `\n\n${startMarker}\n## ${heading}\n${lines.join("\n")}\n${endMarker}\n`;
}

/** Render the delimited links block (authored wikilinks / MEMORY_RELATES). */
export function renderLinksBlock(targets: string[], resolveLink: LinkResolver): string {
  return renderManagedBlock(targets, resolveLink, LINK_BLOCK_START, LINK_BLOCK_END, "Links");
}

/** Render the delimited related block (computed semantic neighbors). */
export function renderRelatedBlock(targets: string[], resolveLink: LinkResolver): string {
  return renderManagedBlock(targets, resolveLink, RELATED_BLOCK_START, RELATED_BLOCK_END, "Related");
}

/** Split a note into its raw frontmatter block (without fences) and the body. */
export function splitFrontmatter(text: string): { frontmatter: string | null; body: string } {
  const match = text.match(FRONTMATTER_RE);
  if (!match) {
    return { frontmatter: null, body: text };
  }
  return { frontmatter: match[1], body: text.slice(match[0].length) };
}

/** Remove one delimited managed block (by its markers) from a body. */
function stripManagedBlock(body: string, startMarker: string, endMarker: string): string {
  const start = body.indexOf(startMarker);
  if (start === -1) {
    return body;
  }
  const endIdx = body.indexOf(endMarker, start);
  const end = endIdx === -1 ? body.length : endIdx + endMarker.length;
  return body.slice(0, start) + body.slice(end);
}

/** Remove the generated links block, returning the user-authored body only. */
export function stripLinksBlock(body: string): string {
  return stripManagedBlock(body, LINK_BLOCK_START, LINK_BLOCK_END).trimEnd();
}

/**
 * The user-visible body of a note: frontmatter removed, and BOTH generated blocks
 * (links and related) removed. Write-back hashes this to tell a real user edit
 * apart from a graph-originated write, so it must strip every managed region.
 */
export function userBody(noteText: string): string {
  let body = splitFrontmatter(noteText).body;
  body = stripManagedBlock(body, LINK_BLOCK_START, LINK_BLOCK_END);
  body = stripManagedBlock(body, RELATED_BLOCK_START, RELATED_BLOCK_END);
  return body.trim();
}

/** Extract wikilink targets from a body (alias, heading and block refs stripped). */
export function extractWikilinks(body: string): string[] {
  const targets = new Set<string>();
  let match: RegExpExecArray | null;
  WIKILINK_RE.lastIndex = 0;
  while ((match = WIKILINK_RE.exec(body)) !== null) {
    const raw = match[1].split("|")[0].split("#")[0].split("^")[0].trim();
    if (raw) {
      targets.add(raw);
    }
  }
  return [...targets];
}

/** Whether `path` sits inside `folder` (or equals it). */
export function isInFolder(path: string, folder: string): boolean {
  if (!folder) {
    return false;
  }
  return path === folder || path.startsWith(`${folder}/`);
}

/**
 * Whether a note is in the write-back capture scope: inside the capture folder
 * (defaulting to the sync folder) OR carrying the capture flag in frontmatter.
 * A note that already carries a doc_id always round-trips and bypasses this gate.
 */
export function isCaptured(
  path: string,
  frontmatter: Record<string, unknown> | null,
  settings: HarnessSyncSettings
): boolean {
  const captureFolder = settings.captureFolder || settings.syncFolder;
  if (isInFolder(path, captureFolder)) {
    return true;
  }
  const flag = settings.captureFlag.trim();
  if (flag && frontmatter && isTruthy(frontmatter[flag])) {
    return true;
  }
  return false;
}

function isTruthy(value: unknown): boolean {
  if (value === true) return true;
  if (typeof value === "string") {
    const v = value.trim().toLowerCase();
    return v === "true" || v === "yes" || v === "1";
  }
  return false;
}

/** Build a YAML frontmatter block (with fences) from a flat record. */
export function buildFrontmatter(fields: Record<string, unknown>): string {
  const lines: string[] = ["---"];
  for (const [key, value] of Object.entries(fields)) {
    if (Array.isArray(value)) {
      if (value.length === 0) {
        lines.push(`${key}: []`);
      } else {
        lines.push(`${key}:`);
        for (const item of value) {
          lines.push(`  - ${yamlScalar(item)}`);
        }
      }
    } else if (value === undefined || value === null || value === "") {
      lines.push(`${key}: ""`);
    } else {
      lines.push(`${key}: ${yamlScalar(value)}`);
    }
  }
  lines.push("---");
  return lines.join("\n") + "\n";
}

function yamlScalar(value: unknown): string {
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  const text = String(value);
  // Quote anything that could be misread as YAML structure.
  if (/^[\w.@/-]+$/.test(text)) {
    return text;
  }
  return `"${text.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}
