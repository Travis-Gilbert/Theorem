/**
 * Categorical-set projection adapter.
 *
 * Pure placement function. Groups atoms by a categorical key and lays each
 * category out as a column, with the category's members stacked down the
 * column. The "show me my knowledge grouped by type / cluster / status"
 * shape: object types, communities, tension states, any discrete partition.
 *
 * Coordinate space is `matrix` (a categorical grid: category on the x axis,
 * member index on the y axis). No D3 layout dependency beyond core grouping;
 * placement is a deterministic grid.
 *
 * Category resolution per atom (first present wins): metadata[categoryKey] ->
 * metadata.category -> metadata.group -> atom.kind -> 'uncategorized'.
 *
 * Determinism: categories are sorted lexicographically and members within a
 * category are sorted by id before placement, so project + terminalState are
 * reproducible regardless of input order. Every atom is placed.
 */

import type { Atom, AtomPosition } from '../atoms/types';
import type {
  ProjectionAdapter,
  ProjectionInput,
  ProjectionOutput,
} from '../substrate/projection';
import { genericTerminalState } from '../substrate/projection';

export interface CategoricalSetProjectionParams {
  /** Metadata key holding each atom's category. Default tries
   *  'category' then 'group', then falls back to atom.kind. */
  categoryKey?: string;
  /** Horizontal gap between category columns, substrate units. Default 160. */
  columnGap?: number;
  /** Vertical gap between members within a column, substrate units. Default 56. */
  rowGap?: number;
}

const CATEGORY_KEY_FALLBACKS = ['category', 'group'] as const;
const UNCATEGORIZED = 'uncategorized';

export const CATEGORICAL_SET_PROJECTION: ProjectionAdapter = {
  id: 'categorical_set',
  label: 'Categorical Set',
  coordinateSpace: 'matrix',
  hostOverlay: 'none',
  supportedAtomKinds: undefined,
  project(input: ProjectionInput): ProjectionOutput {
    const params = (input.host ?? {}) as CategoricalSetProjectionParams;
    const columnGap = positiveOr(params.columnGap, 160);
    const rowGap = positiveOr(params.rowGap, 56);

    const positions = new Map<string, AtomPosition>();
    if (input.atoms.length === 0) {
      return { coordinateSpace: 'matrix', positions };
    }

    // Group atoms by category, members sorted by id.
    const byCategory = new Map<string, string[]>();
    for (const atom of input.atoms) {
      const category = resolveCategory(atom, params.categoryKey);
      const bucket = byCategory.get(category);
      if (bucket === undefined) byCategory.set(category, [atom.id]);
      else bucket.push(atom.id);
    }
    const categories = Array.from(byCategory.keys()).sort((a, b) =>
      a.localeCompare(b),
    );

    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    categories.forEach((category, columnIndex) => {
      const members = byCategory.get(category)!.slice().sort((a, b) => a.localeCompare(b));
      const x = columnIndex * columnGap;
      members.forEach((id, rowIndex) => {
        const y = rowIndex * rowGap;
        positions.set(id, { x, y, space: 'matrix' });
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      });
    });

    return {
      coordinateSpace: 'matrix',
      positions,
      cameraHint: { bounds: { minX, minY, maxX, maxY } },
    };
  },
  terminalState(input) {
    return genericTerminalState(input, 'categorical_set', 'matrix');
  },
};

function resolveCategory(atom: Atom, categoryKey: string | undefined): string {
  const meta = atom.metadata ?? {};
  if (categoryKey !== undefined) {
    const keyed = toLabel(meta[categoryKey]);
    if (keyed !== null) return keyed;
  }
  for (const key of CATEGORY_KEY_FALLBACKS) {
    const candidate = toLabel(meta[key]);
    if (candidate !== null) return candidate;
  }
  if (typeof atom.kind === 'string' && atom.kind.trim() !== '') return atom.kind;
  return UNCATEGORIZED;
}

function toLabel(value: unknown): string | null {
  if (typeof value === 'string' && value.trim() !== '') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  return null;
}

function positiveOr(value: number | undefined, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : fallback;
}
