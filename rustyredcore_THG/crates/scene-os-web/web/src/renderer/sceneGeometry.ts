/**
 * Pure scene geometry: scene-package -> placed atoms + fit transform.
 *
 * This module is DOM-free so the layout math can be unit-tested headlessly
 * (the canvas drawing in `SceneRenderer.ts` consumes its output). It is the
 * Lane B analog of running the projection pipeline: take the scene package
 * Lane A produced, resolve the projection adapter, run it to get world-space
 * positions, and reduce them to screen-space coordinates the canvas paints.
 *
 * Two honesty guarantees live here:
 *
 *   - Unknown projection id -> freeform fallback (via `resolveProjection`),
 *     reported through `fellBack` so the chrome can say so out loud.
 *   - Degenerate placement (every atom at one point, the freeform-with-no-
 *     positions case) -> a deterministic grid so the scene is visible instead
 *     of collapsing to a single dot. Reported through `gridFallback`.
 */

import type { Atom, Relation } from '../atoms/types';
import type { ScenePackageV2 } from '../v2-package';
import { resolveProjection } from '../projections/productionRegistry';

export interface Viewport {
  width: number;
  height: number;
}

export interface WorldPoint {
  x: number;
  y: number;
}

export interface Bounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

export interface SceneLayout {
  atoms: readonly Atom[];
  relations: readonly Relation[];
  /** World-space position per atom id (post grid-fallback). */
  positions: ReadonlyMap<string, WorldPoint>;
  bounds: Bounds;
  coordinateSpace: string;
  projectionId: string;
  projectionLabel: string;
  chromeId: string;
  /** The projection id was unknown; freeform was substituted. */
  fellBack: boolean;
  requestedProjectionId: string;
  /** Placement was degenerate and a synthetic grid was applied. */
  gridFallback: boolean;
}

/** Screen-space affine transform produced by fitting world bounds into a
 *  viewport with uniform scale + centering. */
export interface FitTransform {
  scale: number;
  /** Screen X for a world X. */
  toScreenX(worldX: number): number;
  /** Screen Y for a world Y. */
  toScreenY(worldY: number): number;
}

const MIN_SPAN = 1e-6;

/**
 * Run the scene package through its projection and reduce to placed atoms.
 *
 * `viewport` is handed to the projection so viewport-aware adapters
 * (numeric_series, sankey_flow) scale to the available space.
 */
export function layoutScene(pkg: ScenePackageV2, viewport: Viewport): SceneLayout {
  const { adapter, fellBack, requestedId } = resolveProjection(pkg.projection.id);
  const output = adapter.project({
    atoms: pkg.atoms,
    relations: pkg.relations,
    viewport,
    host: pkg.projection.params,
  });

  const positions = new Map<string, WorldPoint>();
  for (const atom of pkg.atoms) {
    const placed = output.positions.get(atom.id);
    positions.set(atom.id, placed ? { x: placed.x, y: placed.y } : { x: 0, y: 0 });
  }

  let bounds = boundsOf(positions);
  let gridFallback = false;
  if (isDegenerate(bounds) && positions.size > 1) {
    applyGrid(pkg.atoms, positions);
    bounds = boundsOf(positions);
    gridFallback = true;
  }

  return {
    atoms: pkg.atoms,
    relations: pkg.relations,
    positions,
    bounds,
    coordinateSpace: output.coordinateSpace,
    projectionId: adapter.id,
    projectionLabel: adapter.label,
    chromeId: pkg.chrome.id,
    fellBack,
    requestedProjectionId: requestedId,
    gridFallback,
  };
}

/**
 * Fit world bounds into the viewport: uniform scale so the whole scene is
 * visible with `paddingPx` margin, centered. Returns a transform with the
 * world->screen mapping the canvas applies per point.
 */
export function fitTransform(
  bounds: Bounds,
  viewport: Viewport,
  paddingPx: number,
): FitTransform {
  const worldW = Math.max(MIN_SPAN, bounds.maxX - bounds.minX);
  const worldH = Math.max(MIN_SPAN, bounds.maxY - bounds.minY);
  const availW = Math.max(MIN_SPAN, viewport.width - paddingPx * 2);
  const availH = Math.max(MIN_SPAN, viewport.height - paddingPx * 2);

  const scale = Math.min(availW / worldW, availH / worldH);

  // Center the scaled world inside the viewport.
  const worldCx = (bounds.minX + bounds.maxX) / 2;
  const worldCy = (bounds.minY + bounds.maxY) / 2;
  const screenCx = viewport.width / 2;
  const screenCy = viewport.height / 2;

  return {
    scale,
    toScreenX: (worldX: number) => screenCx + (worldX - worldCx) * scale,
    toScreenY: (worldY: number) => screenCy + (worldY - worldCy) * scale,
  };
}

/** Bounding box of a set of world points. Empty -> unit box. */
export function boundsOf(positions: ReadonlyMap<string, WorldPoint>): Bounds {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const point of positions.values()) {
    if (point.x < minX) minX = point.x;
    if (point.x > maxX) maxX = point.x;
    if (point.y < minY) minY = point.y;
    if (point.y > maxY) maxY = point.y;
  }
  if (!Number.isFinite(minX)) return { minX: 0, minY: 0, maxX: 1, maxY: 1 };
  return { minX, minY, maxX, maxY };
}

function isDegenerate(bounds: Bounds): boolean {
  return bounds.maxX - bounds.minX < MIN_SPAN && bounds.maxY - bounds.minY < MIN_SPAN;
}

/**
 * Deterministic grid for degenerate placements. Atoms are sorted by id (the
 * same ordering the projection contract uses) then placed on a near-square
 * grid with unit spacing, so the freeform fallback is visible and stable.
 */
function applyGrid(atoms: readonly Atom[], positions: Map<string, WorldPoint>): void {
  const ids = atoms.map((atom) => atom.id).sort((a, b) => a.localeCompare(b));
  const cols = Math.max(1, Math.ceil(Math.sqrt(ids.length)));
  ids.forEach((id, index) => {
    positions.set(id, { x: index % cols, y: Math.floor(index / cols) });
  });
}
