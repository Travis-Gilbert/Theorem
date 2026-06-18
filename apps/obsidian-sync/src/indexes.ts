// Generated Map-of-Content index notes.
//
// These are plugin-owned files, regenerated every sync from the journal (so they
// list every synced note, not just the batch that changed this pull). Each carries
// `theorem_generated: index` in frontmatter; write-back skips any file with that
// flag, so the indexes never round-trip into the graph.

import type { HarnessSyncSettings } from "./settings";
import { kindFolder } from "./notes";

/** Frontmatter flag marking a plugin-owned generated file. Write-back skips these. */
export const GENERATED_FRONTMATTER_KEY = "theorem_generated";
export const GENERATED_FRONTMATTER_VALUE = "index";

/** A synced note as the index generator needs to see it. */
export interface IndexedNote {
  basename: string;
  title: string;
  kind: string;
  summary: string;
}

export interface GeneratedFile {
  path: string;
  content: string;
}

const FENCE = "```";
const FRONT = `---\n${GENERATED_FRONTMATTER_KEY}: ${GENERATED_FRONTMATTER_VALUE}`;

/**
 * Build the index files for the current synced set. Returns one per-kind index
 * (only when folder-by-kind is on) plus the root Map-of-Content. Returns [] when
 * `generateIndexes` is off.
 */
export function generateIndexFiles(
  notes: IndexedNote[],
  settings: HarnessSyncSettings
): GeneratedFile[] {
  if (!settings.generateIndexes) {
    return [];
  }
  const sync = settings.syncFolder;
  const files: GeneratedFile[] = [];

  // Group notes by their folder (one group under the sync folder when flat).
  const groups = new Map<string, IndexedNote[]>();
  for (const note of notes) {
    const folder = settings.folderByKind ? kindFolder(note.kind) : "";
    const arr = groups.get(folder) ?? [];
    arr.push(note);
    groups.set(folder, arr);
  }

  const sections: Array<{ folder: string; count: number }> = [];

  if (settings.folderByKind) {
    for (const [folder, arr] of [...groups.entries()].sort(([a], [b]) => a.localeCompare(b))) {
      if (!folder) {
        continue;
      }
      const sorted = [...arr].sort((a, b) => a.title.localeCompare(b.title));
      const lines = sorted.map((note) => {
        const alias = note.title && note.title !== note.basename ? `|${note.title}` : "";
        const summary = note.summary.trim() ? `\n  ${note.summary.trim()}` : "";
        return `- [[${note.basename}${alias}]]${summary}`;
      });
      const noun = arr.length === 1 ? "note" : "notes";
      const content =
        `${FRONT}\nkind_folder: ${folder}\n---\n` +
        `# ${folder}\n\n${arr.length} ${noun}.\n\n${lines.join("\n")}\n`;
      files.push({ path: `${sync}/${folder}/_${folder}.md`, content });
      sections.push({ folder, count: arr.length });
    }
  }

  files.push({ path: `${sync}/${settings.indexFileName}.md`, content: rootMap(sync, settings, notes.length, sections) });
  return files;
}

function rootMap(
  sync: string,
  settings: HarnessSyncSettings,
  total: number,
  sections: Array<{ folder: string; count: number }>
): string {
  const noun = total === 1 ? "note" : "notes";
  const sectionLines = sections.length
    ? sections.map((s) => `- [[_${s.folder}|${s.folder}]] (${s.count})`).join("\n")
    : "_(flat layout: folder-by-kind is off)_";

  // Dataview blocks light up only if the user has the Dataview plugin; the static
  // per-kind indexes above work without it. The index notes exclude themselves via
  // the generated flag.
  const recent =
    `${FENCE}dataview\n` +
    `table kind, updated_at as updated\n` +
    `from "${sync}"\n` +
    `where ${GENERATED_FRONTMATTER_KEY} != "${GENERATED_FRONTMATTER_VALUE}"\n` +
    `sort updated_at desc\n` +
    `limit 20\n` +
    `${FENCE}`;
  const byKind =
    `${FENCE}dataview\n` +
    `table length(rows) as Count\n` +
    `from "${sync}"\n` +
    `where ${GENERATED_FRONTMATTER_KEY} != "${GENERATED_FRONTMATTER_VALUE}"\n` +
    `group by kind\n` +
    `${FENCE}`;

  return (
    `${FRONT}\n---\n` +
    `# ${settings.indexFileName}\n\n` +
    `Your Theorem memory, mirrored from the harness (${total} ${noun}). Folders ` +
    `group by kind; identity lives in each note's \`doc_id\`. Regenerated every sync.\n\n` +
    `## Sections\n\n${sectionLines}\n\n` +
    `## Recently updated\n\n${recent}\n\n` +
    `## By kind\n\n${byKind}\n`
  );
}
