/**
 * Scene OS v2 atom substrate types.
 *
 * These mirror `apps/notebook/scene_os/atoms.py`. v2 adds richer per-atom
 * continuous attributes on top of the v1 manifest contract so the substrate
 * can drive smooth morphing transitions across recompiles.
 *
 * v1 ``SceneAtom`` / ``SceneRelation`` continue to live in `../types.ts` and
 * are emitted by the existing recipes/resolvers. v2 projection adapters
 * write the types in this file.
 */

/**
 * Coordinate system a placement function maps atoms into. Every projection
 * adapter declares one of these. The substrate uses the declaration to pick
 * interpolation rules during morph transitions and to decide which host
 * overlays (map tiles, timeline rails, gallery columns) compose around the
 * canvas.
 */
export type CoordinateSpace =
  | 'graph'
  | 'geo'
  | 'timeline'
  | 'rank'
  | 'matrix'
  | 'diagram'
  | 'frame'
  | 'gallery'
  | 'freeform';

export const COORDINATE_SPACES: readonly CoordinateSpace[] = [
  'graph',
  'geo',
  'timeline',
  'rank',
  'matrix',
  'diagram',
  'frame',
  'gallery',
  'freeform',
] as const;

/**
 * Phase an atom is in within a substrate frame. Drives enter/leave
 * animations, terminal-state freezing, and graceful removal.
 */
export type AtomLifecycle = 'entering' | 'present' | 'leaving' | 'terminal';

export const LIFECYCLE_STATES: readonly AtomLifecycle[] = [
  'entering',
  'present',
  'leaving',
  'terminal',
] as const;

/**
 * Placement of one atom in a coordinate space. ``x`` and ``y`` are the
 * canonical 2D substrate coordinates. ``z`` is reserved for substrate depth
 * and may be left at zero. ``space`` records which coordinate space the
 * placement was computed in so the choreographer can interpolate correctly
 * across projection switches.
 */
export interface AtomPosition {
  x: number;
  y: number;
  z?: number;
  space: CoordinateSpace;
}

/**
 * Pointer back to the canonical record an atom was projected from. A single
 * atom may be backed by an Object, an Edge, a Claim, a Source, or an asset.
 * The substrate keeps source refs alongside atoms so user actions can
 * resolve back without going through the manifest.
 */
export interface SourceRef {
  kind: string;
  id: string;
  label?: string;
  metadata?: Record<string, unknown>;
}

/**
 * A single visual element on the substrate.
 *
 * All visual attributes are optional so each projection can choose which
 * dimensions it drives. IDs are stable across patches, recompiles, and
 * projection switches so morph transitions can interpolate atom-to-atom
 * without creating phantom appearances.
 */
export interface Atom {
  id: string;
  kind: string;
  label?: string;
  position?: AtomPosition;
  weight?: number;
  color?: string;
  opacity?: number;
  glyph?: string;
  scale?: number;
  lifecycle: AtomLifecycle;
  metadata?: Record<string, unknown>;
  sourceRefs?: SourceRef[];
}

/**
 * An edge between two atoms. Visual attributes are optional. The substrate
 * uses these to draw or omit links per projection (geo suppresses arrows
 * in favor of geographic adjacency, graph draws every relation as a
 * force-bound link).
 */
export interface Relation {
  id: string;
  sourceId: string;
  targetId: string;
  kind: string;
  weight?: number;
  color?: string;
  opacity?: number;
  glyph?: string;
  lifecycle: AtomLifecycle;
  metadata?: Record<string, unknown>;
  sourceRefs?: SourceRef[];
}

/**
 * Live atom store. The frontend keeps atoms and relations in Maps keyed by
 * ID for O(1) lookup during patch operations. ``version`` is monotonically
 * increased on every applied patch; consumers compare versions to detect
 * stale snapshots without deep-equality.
 */
export interface AtomStore {
  atoms: Map<string, Atom>;
  relations: Map<string, Relation>;
  version: number;
  projectionId?: string;
  chromeId?: string;
  manifestRef?: string;
}

/**
 * Diff applied to a single atom. Every field except ``id`` is optional;
 * only present (i.e. not undefined) fields are applied. To remove an atom,
 * transition it through ``leaving`` first so animations can play, then
 * remove it via ``setAtoms`` on the next compile.
 */
export interface AtomPatch {
  id: string;
  kind?: string;
  label?: string;
  position?: AtomPosition;
  weight?: number;
  color?: string;
  opacity?: number;
  glyph?: string;
  scale?: number;
  lifecycle?: AtomLifecycle;
  metadata?: Record<string, unknown>;
  sourceRefs?: SourceRef[];
}

export interface RelationPatch {
  id: string;
  kind?: string;
  weight?: number;
  color?: string;
  opacity?: number;
  glyph?: string;
  lifecycle?: AtomLifecycle;
  metadata?: Record<string, unknown>;
  sourceRefs?: SourceRef[];
}

// --------------------------------------------------------------------------
// Patch application
// --------------------------------------------------------------------------

/**
 * Returns a new ``Atom`` with patch fields applied. Identity is preserved:
 * the resulting atom has the same ``id`` as the input. Throws if the patch
 * targets a different atom.
 */
export function applyAtomPatch(atom: Atom, patch: AtomPatch): Atom {
  if (patch.id !== atom.id) {
    throw new Error(`AtomPatch id ${patch.id} does not match atom id ${atom.id}`);
  }
  return {
    id: atom.id,
    kind: patch.kind ?? atom.kind,
    label: patch.label ?? atom.label,
    position: patch.position ?? atom.position,
    weight: patch.weight ?? atom.weight,
    color: patch.color ?? atom.color,
    opacity: patch.opacity ?? atom.opacity,
    glyph: patch.glyph ?? atom.glyph,
    scale: patch.scale ?? atom.scale,
    lifecycle: patch.lifecycle ?? atom.lifecycle,
    metadata: patch.metadata ?? atom.metadata,
    sourceRefs: patch.sourceRefs ?? atom.sourceRefs,
  };
}

export function applyRelationPatch(relation: Relation, patch: RelationPatch): Relation {
  if (patch.id !== relation.id) {
    throw new Error(`RelationPatch id ${patch.id} does not match relation id ${relation.id}`);
  }
  return {
    id: relation.id,
    sourceId: relation.sourceId,
    targetId: relation.targetId,
    kind: patch.kind ?? relation.kind,
    weight: patch.weight ?? relation.weight,
    color: patch.color ?? relation.color,
    opacity: patch.opacity ?? relation.opacity,
    glyph: patch.glyph ?? relation.glyph,
    lifecycle: patch.lifecycle ?? relation.lifecycle,
    metadata: patch.metadata ?? relation.metadata,
    sourceRefs: patch.sourceRefs ?? relation.sourceRefs,
  };
}

// --------------------------------------------------------------------------
// Validation
// --------------------------------------------------------------------------

/**
 * Sanity-check an atom before it goes into the substrate. Returns an error
 * message if the atom is malformed, or ``null`` if it's well-formed.
 */
export function validateAtom(atom: Atom): string | null {
  if (!atom.id) {
    return 'Atom.id is required';
  }
  if (atom.opacity !== undefined && (atom.opacity < 0 || atom.opacity > 1)) {
    return `Atom.opacity must be in [0, 1] (atom_id=${atom.id}, value=${atom.opacity})`;
  }
  if (atom.scale !== undefined && atom.scale < 0) {
    return `Atom.scale must be non-negative (atom_id=${atom.id}, value=${atom.scale})`;
  }
  if (!LIFECYCLE_STATES.includes(atom.lifecycle)) {
    return `Atom.lifecycle ${atom.lifecycle} is not a known lifecycle state`;
  }
  if (atom.position && !COORDINATE_SPACES.includes(atom.position.space)) {
    return `Atom.position.space ${atom.position.space} is not a known coordinate space`;
  }
  return null;
}

export function validateRelation(relation: Relation): string | null {
  if (!relation.id) {
    return 'Relation.id is required';
  }
  if (!relation.sourceId || !relation.targetId) {
    return `Relation ${relation.id} requires both sourceId and targetId`;
  }
  if (relation.opacity !== undefined && (relation.opacity < 0 || relation.opacity > 1)) {
    return `Relation.opacity must be in [0, 1] (relation_id=${relation.id}, value=${relation.opacity})`;
  }
  if (!LIFECYCLE_STATES.includes(relation.lifecycle)) {
    return `Relation.lifecycle ${relation.lifecycle} is not a known lifecycle state`;
  }
  return null;
}

// --------------------------------------------------------------------------
// Empty store factory
// --------------------------------------------------------------------------

export function createEmptyAtomStore(): AtomStore {
  return {
    atoms: new Map(),
    relations: new Map(),
    version: 0,
  };
}
