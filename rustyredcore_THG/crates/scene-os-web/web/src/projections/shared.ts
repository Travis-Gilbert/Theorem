/**
 * Shared projection helpers + contract test runner.
 *
 * Every projection adapter must conform to the
 * ``substrateContractAssertions`` shape so a single contract suite can
 * verify them. The helpers below are utilities each adapter can opt into.
 */

import type { Atom, AtomLifecycle, AtomPosition, AtomPatch } from '../atoms/types';
import type { ProjectionAdapter, ProjectionInput, ProjectionOutput } from '../substrate/projection';

// --------------------------------------------------------------------------
// Lifecycle-aware enter / leave
// --------------------------------------------------------------------------

/**
 * Compute lifecycle transition patches from prior to next atom set.
 *
 * - Atoms in next but not prior -> ``entering``.
 * - Atoms in prior but not next -> ``leaving`` (callers should remove
 *   them on the *next* frame after the transition completes).
 * - Atoms in both -> ``present``.
 */
export function computeLifecycleTransitions(
  prior: readonly Atom[],
  next: readonly Atom[],
): AtomPatch[] {
  const priorIds = new Set(prior.map((atom) => atom.id));
  const nextIds = new Set(next.map((atom) => atom.id));
  const patches: AtomPatch[] = [];
  for (const atom of next) {
    const lifecycle: AtomLifecycle = priorIds.has(atom.id) ? 'present' : 'entering';
    patches.push({ id: atom.id, lifecycle });
  }
  for (const atom of prior) {
    if (!nextIds.has(atom.id)) {
      patches.push({ id: atom.id, lifecycle: 'leaving' });
    }
  }
  return patches;
}

// --------------------------------------------------------------------------
// Glyph selection from kind
// --------------------------------------------------------------------------

const KIND_TO_GLYPH: Record<string, string> = {
  source: 'document-page',
  document: 'document-page',
  evidence: 'evidence-pin',
  claim: 'evidence-pin',
  place: 'route-marker',
  region: 'route-marker',
  person: 'person-silhouette',
  building: 'building',
  process_step: 'step-marker',
  shot: 'video-frame',
  image_tile: 'document-page',
  cluster: 'hex',
};

export function defaultGlyphForKind(kind: string): string | undefined {
  return KIND_TO_GLYPH[kind];
}

// --------------------------------------------------------------------------
// Reduced-motion fast path
// --------------------------------------------------------------------------

/**
 * When prefers-reduced-motion is set, projections snap atoms to their
 * target positions immediately rather than animating. This helper
 * collapses an enter/leave + position diff into a single
 * ``setLifecycle('present')`` + ``setAtoms(target)`` operation.
 */
export function reducedMotionSnap(
  positions: ReadonlyMap<string, AtomPosition>,
  atoms: readonly Atom[],
): Atom[] {
  return atoms.map((atom) => ({
    ...atom,
    position: positions.get(atom.id) ?? atom.position,
    lifecycle: 'present' as const,
  }));
}

// --------------------------------------------------------------------------
// Easing
// --------------------------------------------------------------------------

/**
 * Cubic ease-in-out used by morph transitions. Pure function so tests
 * can verify its shape.
 */
export function easeInOutCubic(t: number): number {
  if (t < 0) return 0;
  if (t > 1) return 1;
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

// --------------------------------------------------------------------------
// Accessibility tree
// --------------------------------------------------------------------------

export interface AccessibilityNode {
  id: string;
  label: string;
  kind: string;
  children?: AccessibilityNode[];
}

/**
 * Builds a flat accessibility tree the substrate's offscreen DOM
 * mirror reads. Projection adapters call this to produce the
 * assistive-tech mirror; chrome shells consume it through ARIA props.
 */
export function buildAccessibilityTree(atoms: readonly Atom[]): AccessibilityNode[] {
  return atoms.map((atom) => ({
    id: atom.id,
    label: atom.label ?? atom.id,
    kind: atom.kind,
  }));
}

// --------------------------------------------------------------------------
// Substrate contract assertions
// --------------------------------------------------------------------------

/**
 * Asserts that a projection adapter satisfies the substrate contract:
 * pure function, places every input atom, declares a coordinate space
 * matching its capability declaration. Returns a list of assertion
 * messages; an empty list means the adapter passes.
 *
 * Used by the projection test suite as a single contract sweep across
 * every adapter, so adapters cannot quietly drift from the substrate
 * contract.
 */
export function substrateContractAssertions(
  adapter: ProjectionAdapter,
  input: ProjectionInput,
): string[] {
  const failures: string[] = [];

  const out = adapter.project(input);
  if (out.coordinateSpace !== adapter.coordinateSpace) {
    failures.push(
      `coordinateSpace mismatch: adapter declares ${adapter.coordinateSpace}, output declares ${out.coordinateSpace}`,
    );
  }
  for (const atom of input.atoms) {
    if (!out.positions.has(atom.id)) {
      failures.push(`adapter missing position for atom ${atom.id}`);
      break;
    }
  }
  // Purity: re-running the adapter on the same input must produce the
  // same coordinateSpace + same set of position keys.
  const second = adapter.project(input);
  if (second.coordinateSpace !== out.coordinateSpace) {
    failures.push(`adapter is non-deterministic on coordinateSpace`);
  }
  if (second.positions.size !== out.positions.size) {
    failures.push(`adapter is non-deterministic on positions.size`);
  }
  return failures;
}

// --------------------------------------------------------------------------
// Terminal state SVG emission
// --------------------------------------------------------------------------

/**
 * Builds a stable SVG snapshot of an atom set + their positions.
 * Atoms with no position are skipped. Source refs are encoded as
 * ``data-source-*`` attributes so the SVG is self-citing.
 *
 * The SVG is INTENTIONALLY simple: single ``<g>`` of points (the
 * substrate's default glyph) and ``<line>`` segments for relations.
 * Stage 06 (Choreographer + terminalState) extends this with chrome-
 * declared decorations.
 */
export function buildTerminalSvg(
  atoms: readonly Atom[],
  relations: readonly { id: string; sourceId: string; targetId: string }[],
  positions: ReadonlyMap<string, AtomPosition>,
  viewport: { width: number; height: number },
): string {
  const lines: string[] = [];
  lines.push(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${viewport.width} ${viewport.height}">`,
  );

  // Relations first so atoms paint over them.
  for (const relation of relations) {
    const a = positions.get(relation.sourceId);
    const b = positions.get(relation.targetId);
    if (!a || !b) continue;
    lines.push(
      `<line x1="${roundTo(a.x, 2)}" y1="${roundTo(a.y, 2)}" x2="${roundTo(
        b.x,
        2,
      )}" y2="${roundTo(b.y, 2)}" stroke="rgba(120,120,130,0.3)" stroke-width="1" />`,
    );
  }

  for (const atom of atoms) {
    const pos = positions.get(atom.id);
    if (!pos) continue;
    const sourceAttrs = atom.sourceRefs
      ?.map((ref) => `data-source-${ref.kind}="${escapeAttr(ref.id)}"`)
      .join(' ') ?? '';
    lines.push(
      `<circle cx="${roundTo(pos.x, 2)}" cy="${roundTo(pos.y, 2)}" r="4" fill="${escapeAttr(
        atom.color ?? '#444',
      )}" data-atom-id="${escapeAttr(atom.id)}" ${sourceAttrs}><title>${escapeAttr(
        atom.label ?? atom.id,
      )}</title></circle>`,
    );
  }

  lines.push('</svg>');
  return lines.join('');
}

function roundTo(value: number, digits: number): number {
  const factor = 10 ** digits;
  return Math.round(value * factor) / factor;
}

function escapeAttr(value: string): string {
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

export type ProjectionOutputForTest = ProjectionOutput;
