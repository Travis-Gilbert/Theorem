// Pull-time kind filtering.
//
// A tenant feed can be dominated by one machine-written kind (for this graph,
// `orchestrate` agent-coordination exhaust: ~185 of ~236 docs at time of
// writing). The read endpoint does not honor a server-side kind filter, so the
// plugin filters client-side before writing notes, keeping the vault to the
// real memory (solution / feedback / postmortem / encode / decision / note).
//
// This is a PULL concern only: it decides which graph docs become notes. It
// never touches write-back, where a hand-written note's `kind` is authored by
// the user.

import type { HarnessDoc } from "./types";
import type { HarnessSyncSettings } from "./settings";

/** The two settings fields this module reads. */
export type KindFilterSettings = Pick<HarnessSyncSettings, "includeKinds" | "excludeKinds">;

/** Normalize a kind token for comparison: trim + lowercase. */
export function normalizeKind(kind: string): string {
  return kind.trim().toLowerCase();
}

/**
 * Parse a user-entered list ("Orchestrate, solution  feedback") into normalized,
 * de-duplicated kind tokens. Commas, spaces, and newlines all separate; blanks
 * are dropped. Kinds are single tokens, so whitespace is always a separator.
 */
export function parseKindList(value: string): string[] {
  const seen = new Set<string>();
  for (const token of value.split(/[\s,]+/)) {
    const norm = normalizeKind(token);
    if (norm) {
      seen.add(norm);
    }
  }
  return [...seen];
}

/** Render a kind list back into the comma-separated form the settings UI shows. */
export function formatKindList(list: string[] | undefined): string {
  return (list ?? []).join(", ");
}

/**
 * Decide whether a doc of `kind` should be pulled.
 *
 * - `includeKinds` is an allowlist. If non-empty, only kinds in it pass.
 * - `excludeKinds` is a denylist. Any kind in it is dropped.
 * - Exclude wins over include: a kind in both lists is dropped.
 * - Empty/undefined lists are no-ops (an empty allowlist allows everything).
 *
 * Comparison is case-insensitive and whitespace-insensitive.
 */
export function kindAllowed(
  kind: string,
  includeKinds: string[] | undefined,
  excludeKinds: string[] | undefined
): boolean {
  const k = normalizeKind(kind);
  const include = (includeKinds ?? []).map(normalizeKind).filter(Boolean);
  const exclude = (excludeKinds ?? []).map(normalizeKind).filter(Boolean);
  if (include.length > 0 && !include.includes(k)) {
    return false;
  }
  if (exclude.includes(k)) {
    return false;
  }
  return true;
}

/** Filter a doc batch to the kinds the settings allow. */
export function filterDocsByKind(docs: HarnessDoc[], settings: KindFilterSettings): HarnessDoc[] {
  return docs.filter((doc) => kindAllowed(doc.kind, settings.includeKinds, settings.excludeKinds));
}
