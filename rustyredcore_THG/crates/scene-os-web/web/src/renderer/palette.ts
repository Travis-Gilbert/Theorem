/**
 * Visual vocabulary for the Theorem SceneOS renderer.
 *
 * The bundle draws on a single 2D canvas. Its vocabulary is deliberately
 * continuous with two existing surfaces so the scene page feels like part of
 * the same browser, not a parallel language:
 *
 *   1. The substrate baseline (`substrate/projection.ts` genericTerminalState):
 *      relations are lines, atoms are filled marks; default relation stroke
 *      `#9aa0a6`, default atom fill `#3c5572`.
 *   2. The browser chrome (`rustyred-web/src/serp.html`): the studio-journal
 *      palette: paper `#f4f1ea`, ink `#2A2823`, ink-soft `#6b665c`, terracotta
 *      accent `#b45a2d`, JetBrains Mono: and the project's section colors
 *      (terracotta / teal / gold / green).
 *
 * Atom kind drives color and glyph shape (mirroring `projections/shared.ts`
 * `defaultGlyphForKind`). An atom's own `color` (sanitized hex) always wins
 * over the kind default, so a director that paints atoms keeps control.
 */

export const PAPER = '#f4f1ea';
export const PAPER_SOFT = '#ece8df';
export const INK = '#2A2823';
export const INK_SOFT = '#6b665c';
export const RULE = '#d9d4c8';
export const ACCENT = '#b45a2d';
export const MONO = '"JetBrains Mono", ui-monospace, "SFMono-Regular", Menlo, monospace';

/** Substrate baseline defaults (genericTerminalState). */
export const DEFAULT_ATOM_FILL = '#3c5572';
export const DEFAULT_RELATION_STROKE = '#9aa0a6';

/**
 * Atom kind -> fill color. Built from the project's section-color language
 * (terracotta = evidence/argument, teal = source/document, gold = place/cluster,
 * green = person/agent) so kinds read consistently with the rest of the site.
 */
const KIND_COLOR: Readonly<Record<string, string>> = {
  source: '#2D5F6B',
  document: '#2D5F6B',
  image_tile: '#3c6470',
  evidence: '#B45A2D',
  claim: '#B45A2D',
  argument: '#B45A2D',
  process_step: '#9c5a36',
  step: '#9c5a36',
  shot: '#6b665c',
  place: '#C49A4A',
  region: '#C49A4A',
  cluster: '#C49A4A',
  person: '#5A7A4A',
  agent: '#5A7A4A',
  building: '#8a6d4a',
  'patent-node': '#2A2823',
};

/** Glyph name (from shared.ts defaultGlyphForKind) -> draw shape keyword. */
const GLYPH_SHAPE: Readonly<Record<string, ShapeKind>> = {
  'document-page': 'page',
  'evidence-pin': 'pin',
  'route-marker': 'pin',
  'person-silhouette': 'person',
  building: 'building',
  'step-marker': 'square',
  'video-frame': 'frame',
  hex: 'hex',
};

export type ShapeKind =
  | 'circle'
  | 'page'
  | 'pin'
  | 'person'
  | 'building'
  | 'square'
  | 'frame'
  | 'hex';

const HEX_RE = /^#[0-9a-fA-F]{3}([0-9a-fA-F]{3})?$/;

/** Accept only hex colors; reject anything else so a crafted atom.color
 *  literal cannot smuggle a non-color string into canvas fillStyle. */
export function sanitizeColor(input: string | undefined): string | null {
  if (input === undefined) return null;
  const trimmed = input.trim();
  return HEX_RE.test(trimmed) ? trimmed : null;
}

/** Resolve an atom's fill: explicit hex color, else kind color, else the
 *  substrate slate default. */
export function atomFill(kind: string, color: string | undefined): string {
  return sanitizeColor(color) ?? KIND_COLOR[kind] ?? DEFAULT_ATOM_FILL;
}

/** Resolve a relation's stroke: explicit hex color, else the substrate
 *  neutral default. */
export function relationStroke(color: string | undefined): string {
  return sanitizeColor(color) ?? DEFAULT_RELATION_STROKE;
}

/** Map a glyph name to a draw-shape keyword. Unknown / absent glyphs render
 *  as a plain circle (the substrate baseline mark). */
export function shapeForGlyph(glyph: string | undefined): ShapeKind {
  if (glyph === undefined) return 'circle';
  return GLYPH_SHAPE[glyph] ?? 'circle';
}
