/**
 * Numeric-series projection adapter.
 *
 * Pure placement function. Plots atoms as a value series: x is the atom's
 * position along an ordering dimension (a numeric/time key, or sorted index
 * when no key is present), y is the atom's numeric value. Wraps d3-scale's
 * linear scales as the value->pixel math, in the `rank` coordinate space.
 *
 * This is the projection for "show me X over time / across a numeric axis"
 * query shapes (metrics, trends, ranked values). If the scene carries
 * relations between consecutive points, the substrate draws them as the
 * connecting line; otherwise it renders as a scatter of placed points.
 *
 * Value resolution per atom (first present wins): metadata[valueKey] ->
 * metadata.value -> atom.weight -> 0. Order resolution: metadata[orderKey] ->
 * metadata.order -> metadata.x -> metadata.t -> sorted index.
 *
 * Determinism: atoms are sorted by (orderValue, id) before scaling, and
 * d3 linear scales are pure, so project + terminalState are reproducible.
 * Higher value maps to smaller y (up the screen), matching chart convention.
 */

import { scaleLinear } from 'd3-scale';

import type { Atom, AtomPosition } from '../atoms/types';
import type {
  ProjectionAdapter,
  ProjectionInput,
  ProjectionOutput,
} from '../substrate/projection';
import { genericTerminalState } from '../substrate/projection';

export interface NumericSeriesProjectionParams {
  /** Metadata key holding each atom's numeric value (y). Default 'value'. */
  valueKey?: string;
  /** Metadata key holding each atom's ordering value (x). When absent on all
   *  atoms, the sorted index is used. Default tries 'order','x','t' in turn. */
  orderKey?: string;
  /** Pixel padding inside the viewport so end points are not flush to the
   *  edge. Default 48. */
  padding?: number;
}

const ORDER_KEY_FALLBACKS = ['order', 'x', 't'] as const;

export const NUMERIC_SERIES_PROJECTION: ProjectionAdapter = {
  id: 'numeric_series',
  label: 'Numeric Series',
  coordinateSpace: 'rank',
  hostOverlay: 'none',
  supportedAtomKinds: undefined,
  project(input: ProjectionInput): ProjectionOutput {
    const params = (input.host ?? {}) as NumericSeriesProjectionParams;
    const padding = positiveOr(params.padding, 48);
    const positions = new Map<string, AtomPosition>();
    if (input.atoms.length === 0) {
      return { coordinateSpace: 'rank', positions };
    }

    const width = positiveOr(input.viewport.width, 1280);
    const height = positiveOr(input.viewport.height, 720);

    // Resolve value + order for each atom.
    const rows = input.atoms.map((atom) => ({
      id: atom.id,
      value: resolveValue(atom, params.valueKey),
      order: resolveOrder(atom, params.orderKey),
    }));

    // Deterministic ordering: by resolved order, then id. Atoms without an
    // explicit order fall back to this sorted index for their x.
    rows.sort((a, b) => {
      const ao = a.order ?? Number.POSITIVE_INFINITY;
      const bo = b.order ?? Number.POSITIVE_INFINITY;
      if (ao !== bo) return ao - bo;
      return a.id.localeCompare(b.id);
    });

    const hasExplicitOrder = rows.some((r) => r.order !== null);

    // x domain: explicit order values, else index 0..n-1.
    const xScale = scaleLinear()
      .domain(
        hasExplicitOrder
          ? extent(rows.map((r) => r.order ?? 0))
          : [0, Math.max(1, rows.length - 1)],
      )
      .range([padding, width - padding]);

    // y domain: value range. Higher value -> smaller y (top of screen).
    const values = rows.map((r) => r.value);
    const yScale = scaleLinear()
      .domain(extent(values))
      .range([height - padding, padding]);

    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    rows.forEach((row, index) => {
      const x = hasExplicitOrder ? xScale(row.order ?? 0) : xScale(index);
      const y = yScale(row.value);
      positions.set(row.id, { x, y, space: 'rank' });
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (y < minY) minY = y;
      if (y > maxY) maxY = y;
    });

    return {
      coordinateSpace: 'rank',
      positions,
      cameraHint: { bounds: { minX, minY, maxX, maxY } },
    };
  },
  terminalState(input) {
    return genericTerminalState(input, 'numeric_series', 'rank');
  },
};

function resolveValue(atom: Atom, valueKey: string | undefined): number {
  const meta = atom.metadata ?? {};
  const keyed = valueKey !== undefined ? toNumber(meta[valueKey]) : null;
  if (keyed !== null) return keyed;
  const value = toNumber(meta.value);
  if (value !== null) return value;
  if (typeof atom.weight === 'number' && Number.isFinite(atom.weight)) {
    return atom.weight;
  }
  return 0;
}

function resolveOrder(atom: Atom, orderKey: string | undefined): number | null {
  const meta = atom.metadata ?? {};
  if (orderKey !== undefined) {
    const keyed = toNumber(meta[orderKey]);
    if (keyed !== null) return keyed;
  }
  for (const key of ORDER_KEY_FALLBACKS) {
    const candidate = toNumber(meta[key]);
    if (candidate !== null) return candidate;
  }
  return null;
}

function toNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
}

/** Min/max of a non-empty number array as a [min, max] domain. Pads a flat
 *  domain (all equal) by ±1 so the scale does not collapse to a point. */
function extent(values: readonly number[]): [number, number] {
  let min = Infinity;
  let max = -Infinity;
  for (const v of values) {
    if (v < min) min = v;
    if (v > max) max = v;
  }
  if (!Number.isFinite(min) || !Number.isFinite(max)) return [0, 1];
  if (min === max) return [min - 1, max + 1];
  return [min, max];
}

function positiveOr(value: number | undefined, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : fallback;
}
