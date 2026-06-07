import type { HarnessSyncSettings } from "./settings";
import type { HarnessDoc } from "./types";

// The generated links block is delimited by Obsidian comment markers so it renders
// invisibly, yet the wikilinks between the markers stay clickable and show in the
// graph view. Write-back strips this block to recover the user's own body.
export const LINK_BLOCK_START = "%% theorem:links:start %%";
export const LINK_BLOCK_END = "%% theorem:links:end %%";

const FRONTMATTER_RE = /^---\n([\s\S]*?)\n---\n?/;
const WIKILINK_RE = /\[\[([^\]]+)\]\]/g;

/** Resolve a link target doc_id to the basename + title of its note, if known. */
export type LinkResolver = (target: string) => { basename: string; title: string } | null;

/** A stable, human-readable, collision-free note basename: slug(title)-shortid. */
export function noteBasename(title: string, docId: string): string {
  const slug = slugify(title) || "untitled";
  return `${slug}-${shortDocId(docId)}`;
}

export function notePath(folder: string, basename: string): string {
  const prefix = folder ? `${folder}/` : "";
  return `${prefix}${basename}.md`;
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
  return `${frontmatter}\n${body}${links}`;
}

/** Render the delimited links block, or an empty string when there are no links. */
export function renderLinksBlock(targets: string[], resolveLink: LinkResolver): string {
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
  return `\n\n${LINK_BLOCK_START}\n## Links\n${lines.join("\n")}\n${LINK_BLOCK_END}\n`;
}

/** Split a note into its raw frontmatter block (without fences) and the body. */
export function splitFrontmatter(text: string): { frontmatter: string | null; body: string } {
  const match = text.match(FRONTMATTER_RE);
  if (!match) {
    return { frontmatter: null, body: text };
  }
  return { frontmatter: match[1], body: text.slice(match[0].length) };
}

/** Remove the generated links block, returning the user-authored body only. */
export function stripLinksBlock(body: string): string {
  const start = body.indexOf(LINK_BLOCK_START);
  if (start === -1) {
    return body.trimEnd();
  }
  const endMarker = body.indexOf(LINK_BLOCK_END, start);
  const end = endMarker === -1 ? body.length : endMarker + LINK_BLOCK_END.length;
  return (body.slice(0, start) + body.slice(end)).trimEnd();
}

/** The user-visible body of a note: frontmatter removed, generated links removed. */
export function userBody(noteText: string): string {
  return stripLinksBlock(splitFrontmatter(noteText).body).trim();
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
