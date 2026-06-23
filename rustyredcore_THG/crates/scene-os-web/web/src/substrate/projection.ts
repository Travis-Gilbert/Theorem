/**
 * Projection contract.
 *
 * A projection is a function ``Atoms → Placement`` in one
 * ``CoordinateSpace``. The substrate handles rendering; projections
 * own placement only. This file defines the contract every projection
 * adapter must satisfy.
 *
 * Stages 03 and 04 implement the actual projection adapters (geo,
 * graph, diagram, cinematic, matrix, image). Stage 00 ships the
 * contract so those adapters can be authored against it without
 * additional discovery work.
 */

import type { Atom, AtomPosition, CoordinateSpace, Relation } from '../atoms/types';

/**
 * Inputs to a projection invocation. ``viewport`` describes the
 * substrate canvas size in pixels so projections can scale their
 * output to the available space. ``host`` carries projection-specific
 * configuration (e.g. map zoom, timeline range, matrix axes) that the
 * compiler resolved from the manifest.
 */
export interface ProjectionInput {
  atoms: readonly Atom[];
  relations: readonly Relation[];
  viewport: {
    width: number;
    height: number;
  };
  host?: Record<string, unknown>;
}

/**
 * Output of a projection invocation. The substrate consumes
 * ``positions`` to drive the canvas. ``hostOverlay`` lets a projection
 * declare a host library (MapLibre tiles, Remotion timeline, React
 * Flow lanes) that should compose with the canvas; the chrome shell
 * mounts the overlay around the substrate.
 *
 * ``cameraHint`` is a non-binding suggestion to the choreographer
 * (e.g. "fit to bounds X..Y"); the choreographer is the source of
 * truth for camera placement so transitions can morph between
 * projections.
 */
export interface ProjectionOutput {
  coordinateSpace: CoordinateSpace;
  positions: Map<string, AtomPosition>;
  hostOverlay?: ProjectionHostOverlay;
  cameraHint?: ProjectionCameraHint;
}

/**
 * Optional host overlay. The chrome shell mounts the overlay's React
 * component (or canvas / SVG) around the substrate; the overlay reads
 * the same atom store so its content stays in sync. Examples:
 * MapLibre raster tiles below the substrate, a Remotion timeline rail
 * above it, a React Flow lane gutter to the left.
 *
 * The substrate never owns the overlay; chrome shells do.
 */
export interface ProjectionHostOverlay {
  kind: 'maplibre' | 'remotion' | 'react-flow' | 'gallery' | 'matrix-gutters' | 'none';
  config: Record<string, unknown>;
}

export interface ProjectionCameraHint {
  bounds?: {
    minX: number;
    minY: number;
    maxX: number;
    maxY: number;
  };
  focusAtomId?: string;
  zoom?: number;
}

/**
 * Terminal state artifact emitted by a projection at pause / end /
 * save / explicit freeze. Stage 06 / Task 4 of the v2 plan.
 *
 * Must be deterministic from ``atoms``, ``relations``, and ``params``:
 * calling ``terminalState`` twice with identical inputs MUST produce
 * byte-identical ``svg`` and JSON-equal ``json``. The substrate's
 * Stage 06 persistence path hashes the SVG to dedupe artifact saves.
 *
 * The artifact is the source of truth for "the world's greatest
 * diagram" use case: pause the scene, freeze the current placement,
 * persist it as a citable SVG + JSON payload with full provenance.
 *
 * ``sourceRefs`` aggregates every atom's sourceRefs by (kind, id) so
 * the diagram is citable as a single artifact. Duplicates are removed.
 * ``caption`` is an optional one-line description the chrome shell
 * renders alongside the snapshot.
 */
export interface TerminalState {
  svg: string;
  json: TerminalArtifactJson;
  sourceRefs: ReadonlyArray<{ kind: string; id: string }>;
  caption?: string;
}

export interface TerminalArtifactJson {
  /** Projection id that produced the artifact. */
  projectionId: string;
  /** Coordinate space the atoms were placed in. */
  coordinateSpace: CoordinateSpace;
  /** Per-atom snapshot: id + kind + label + position. Excludes
   *  attributes that do not contribute to the visual diagram
   *  (lifecycle, weights, metadata) so the JSON stays small and
   *  hashable. */
  atoms: ReadonlyArray<{
    id: string;
    kind: string;
    label?: string;
    position?: { x: number; y: number; z?: number };
  }>;
  /** Per-relation snapshot: id + source + target + kind. */
  relations: ReadonlyArray<{
    id: string;
    sourceId: string;
    targetId: string;
    kind: string;
  }>;
}

export interface TerminalStateInput {
  atoms: readonly Atom[];
  relations: readonly Relation[];
  /** The projection's host params at the moment of emission. Passed
   *  through to the JSON output for provenance. Per-projection
   *  overrides may use this to render projection-specific decoration
   *  (e.g. matrix gridlines, geo bbox). */
  params?: Record<string, unknown>;
  /** Optional caption override. When omitted, projections derive one
   *  from their params (e.g. "Step 3 of 7" for cinematic). */
  caption?: string;
}

/**
 * The contract every projection adapter must satisfy.
 *
 * Adapters are pure functions: same inputs produce same outputs. They
 * MUST NOT mutate the input atoms / relations. They MUST return a
 * placement for every atom (use position {x:0,y:0,space:'freeform'}
 * for atoms that the projection chooses not to position; the
 * substrate keeps them in the store but they animate to a default
 * cluster).
 *
 * Adapters MAY return more positions than the input contains: it is
 * legal to materialize lane / row / column markers as virtual atoms.
 * The substrate ignores positions whose ID is not in the store.
 */
export interface ProjectionAdapter {
  id: string;
  label: string;
  coordinateSpace: CoordinateSpace;
  /**
   * Optional list of atom kinds this projection can place. The
   * compiler uses this to short-circuit projection selection: if a
   * scene has only kinds the projection cannot place, it falls back
   * to the next candidate. ``undefined`` means "places any kind".
   */
  supportedAtomKinds?: readonly string[];
  /**
   * Optional declaration of host overlay this projection mounts. The
   * chrome shell uses this to validate the manifest's chrome
   * selection: incompatible chrome + projection combinations fail
   * validation at compile time, not runtime.
   */
  hostOverlay?: ProjectionHostOverlay['kind'];
  project(input: ProjectionInput): ProjectionOutput;
  /**
   * Emit a terminal-state artifact for the supplied atoms + relations.
   * Called by the chrome's pause / freeze / save actions. MUST be
   * deterministic from the inputs: same inputs produce byte-identical
   * SVG and JSON-equal JSON. See ``TerminalState`` JSDoc for the full
   * contract.
   *
   * Stage 06 / Task 4 of the v2 plan. Projections may override the
   * default shared rendering (see ``shared.ts:genericTerminalState``)
   * to add projection-specific decoration (e.g. matrix gridlines).
   */
  terminalState(input: TerminalStateInput): TerminalState;
}

/**
 * Trivial pass-through projection used by tests and as the safety net
 * when no other projection accepts the atoms. It places atoms in
 * ``freeform`` space using whatever position they already carry, or
 * the origin if they don't.
 */
export const FREEFORM_PROJECTION: ProjectionAdapter = {
  id: 'freeform',
  label: 'Freeform pass-through',
  coordinateSpace: 'freeform',
  project({ atoms }) {
    const positions = new Map<string, AtomPosition>();
    for (const atom of atoms) {
      positions.set(atom.id, atom.position ?? { x: 0, y: 0, space: 'freeform' });
    }
    return {
      coordinateSpace: 'freeform',
      positions,
    };
  },
  terminalState(input) {
    return genericTerminalState(input, 'freeform', 'freeform');
  },
};

/**
 * Default terminal-state generator. Renders a deterministic SVG of the
 * supplied atoms (as circles) and relations (as straight lines)
 * computed from a viewBox derived from the atom position bounding box.
 * Most projection adapters can use this directly; richer projections
 * (matrix gridlines, geo bbox, cinematic timeline rail) override and
 * compose richer decoration on top.
 *
 * Determinism rules:
 *   * Atoms are sorted by id before iteration so the SVG element order
 *     is deterministic regardless of input order.
 *   * Relations are sorted by (sourceId, targetId, id).
 *   * Number formatting uses ``Number.prototype.toFixed(2)`` so the
 *     output does not vary with platform float precision.
 *   * No timestamp, no random data, no environment-derived values.
 */
export function genericTerminalState(
  input: TerminalStateInput,
  projectionId: string,
  coordinateSpace: CoordinateSpace,
): TerminalState {
  const atomsById = new Map<string, Atom>();
  for (const atom of input.atoms) atomsById.set(atom.id, atom);

  const sortedAtoms = Array.from(input.atoms).sort((a, b) =>
    a.id.localeCompare(b.id),
  );
  const sortedRelations = Array.from(input.relations).sort((a, b) => {
    if (a.sourceId !== b.sourceId) return a.sourceId.localeCompare(b.sourceId);
    if (a.targetId !== b.targetId) return a.targetId.localeCompare(b.targetId);
    return a.id.localeCompare(b.id);
  });

  // Compute bounding box from positioned atoms. Falls back to a unit
  // viewBox when no atoms have positions.
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let anyPositioned = false;
  for (const atom of sortedAtoms) {
    if (atom.position === undefined) continue;
    anyPositioned = true;
    if (atom.position.x < minX) minX = atom.position.x;
    if (atom.position.x > maxX) maxX = atom.position.x;
    if (atom.position.y < minY) minY = atom.position.y;
    if (atom.position.y > maxY) maxY = atom.position.y;
  }
  if (!anyPositioned) {
    minX = 0; minY = 0; maxX = 1; maxY = 1;
  }
  // Pad the viewBox by 5% on each side so atom circles do not clip.
  const padX = Math.max(0.001, (maxX - minX) * 0.05);
  const padY = Math.max(0.001, (maxY - minY) * 0.05);
  const vbMinX = minX - padX;
  const vbMinY = minY - padY;
  const vbWidth = (maxX - minX) + padX * 2;
  const vbHeight = (maxY - minY) + padY * 2;

  // Circle radius scales with the viewBox so large coordinate ranges
  // (geo lat/lng) and small ones (matrix cells) both render visibly.
  const radius = Math.max(vbWidth, vbHeight) * 0.012;
  const strokeWidth = radius * 0.4;

  const parts: string[] = [];
  parts.push(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${num(vbMinX)} ${num(vbMinY)} ${num(vbWidth)} ${num(vbHeight)}" data-projection="${escapeAttr(projectionId)}">`,
  );
  // Relations drawn first so atoms render on top.
  for (const rel of sortedRelations) {
    const source = atomsById.get(rel.sourceId);
    const target = atomsById.get(rel.targetId);
    if (
      source === undefined ||
      target === undefined ||
      source.position === undefined ||
      target.position === undefined
    ) {
      continue;
    }
    const color = sanitizeColor(rel.color) ?? '#9aa0a6';
    parts.push(
      `<line x1="${num(source.position.x)}" y1="${num(source.position.y)}" x2="${num(target.position.x)}" y2="${num(target.position.y)}" stroke="${color}" stroke-width="${num(strokeWidth)}" data-relation-id="${escapeAttr(rel.id)}" />`,
    );
  }
  for (const atom of sortedAtoms) {
    if (atom.position === undefined) continue;
    const fill = sanitizeColor(atom.color) ?? '#3c5572';
    const labelAttr = atom.label !== undefined ? ` data-label="${escapeAttr(atom.label)}"` : '';
    parts.push(
      `<circle cx="${num(atom.position.x)}" cy="${num(atom.position.y)}" r="${num(radius)}" fill="${fill}" data-atom-id="${escapeAttr(atom.id)}"${labelAttr} />`,
    );
  }
  parts.push('</svg>');

  // Aggregate source refs across all atoms, dedup by (kind, id).
  const refKey = (ref: { kind: string; id: string }) => `${ref.kind}::${ref.id}`;
  const seen = new Set<string>();
  const sourceRefs: { kind: string; id: string }[] = [];
  for (const atom of sortedAtoms) {
    for (const ref of atom.sourceRefs ?? []) {
      const key = refKey(ref);
      if (seen.has(key)) continue;
      seen.add(key);
      sourceRefs.push({ kind: ref.kind, id: ref.id });
    }
  }

  const json: TerminalArtifactJson = {
    projectionId,
    coordinateSpace,
    atoms: sortedAtoms.map((atom) => ({
      id: atom.id,
      kind: atom.kind,
      label: atom.label,
      position: atom.position
        ? { x: atom.position.x, y: atom.position.y, z: atom.position.z }
        : undefined,
    })),
    relations: sortedRelations.map((rel) => ({
      id: rel.id,
      sourceId: rel.sourceId,
      targetId: rel.targetId,
      kind: rel.kind,
    })),
  };

  return {
    svg: parts.join(''),
    json,
    sourceRefs,
    caption: input.caption,
  };
}

/** Format a number with two decimal places for deterministic SVG. */
function num(value: number): string {
  if (!Number.isFinite(value)) return '0';
  return value.toFixed(2);
}

/** Sanitize a color string for SVG output. Only accepts hex
 *  (#RGB or #RRGGBB); other forms (rgb(), hsl(), named) are dropped
 *  so the artifact cannot be a vector for injection via a crafted
 *  color literal. Returns null for unparsable input; the caller
 *  supplies a default. */
function sanitizeColor(input: string | undefined): string | null {
  if (input === undefined) return null;
  const trimmed = input.trim();
  if (!/^#[0-9a-fA-F]{3}([0-9a-fA-F]{3})?$/.test(trimmed)) return null;
  return trimmed;
}

/** Escape a value for use in an SVG attribute (double-quoted). Replaces
 *  ``&``, ``<``, ``>``, ``"``, and ``'`` with their entity references.
 *  Defends against atom labels containing injection-shaped substrings. */
function escapeAttr(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}
