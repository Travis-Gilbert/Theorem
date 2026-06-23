/**
 * Layered-flow projection adapter.
 *
 * Pure placement function. Lays a directed relation graph out in layers by
 * longest-path depth (flow direction left -> right), stacking each layer's
 * nodes down the column. This is the placement a flow / sankey / provenance
 * view needs: "source -> claim -> conclusion", "evidence -> derivation ->
 * result". The substrate draws the relations as the connecting flow.
 *
 * This is the PLACEMENT half of the catalog's sankey-flow slot. It does not
 * compute sankey ribbon geometry (node rectangles sized by throughput, link
 * widths by flow): that needs the d3-sankey dependency and is a future
 * decoration on top of this layering. Named `flow_layered` rather than
 * `sankey_flow` so the richer sankey slot stays honestly open.
 *
 * Layering is longest-path from roots (nodes with no incoming relation):
 * layer(n) = 0 when n has no parents, else max(layer(parent)+1). Cycles are
 * tolerated: a back-edge to a node currently on the recursion stack does not
 * extend depth, so a cyclic relation graph still lays out as a clean DAG.
 *
 * Coordinate space is `diagram` (a structured layout), matching tree-hierarchy
 * so the choreographer can morph between the two. Determinism: nodes and
 * parents are sorted by id, so the longest-path memoization and per-layer
 * stacking are reproducible; project + terminalState are byte-stable.
 */

import type { AtomPosition } from '../atoms/types';
import type {
  ProjectionAdapter,
  ProjectionInput,
  ProjectionOutput,
} from '../substrate/projection';
import { genericTerminalState } from '../substrate/projection';

export interface FlowLayeredProjectionParams {
  /** Horizontal gap between layers (flow direction), substrate units.
   *  Default 200. */
  layerGap?: number;
  /** Vertical gap between stacked nodes within a layer, substrate units.
   *  Default 64. */
  nodeGap?: number;
}

export const FLOW_LAYERED_PROJECTION: ProjectionAdapter = {
  id: 'flow_layered',
  label: 'Layered Flow',
  coordinateSpace: 'diagram',
  hostOverlay: 'none',
  supportedAtomKinds: undefined,
  project(input: ProjectionInput): ProjectionOutput {
    const params = (input.host ?? {}) as FlowLayeredProjectionParams;
    const layerGap = positiveOr(params.layerGap, 200);
    const nodeGap = positiveOr(params.nodeGap, 64);

    const positions = new Map<string, AtomPosition>();
    if (input.atoms.length === 0) {
      return { coordinateSpace: 'diagram', positions };
    }

    const nodeIds = Array.from(input.atoms, (atom) => atom.id).sort((a, b) =>
      a.localeCompare(b),
    );
    const nodeIdSet = new Set(nodeIds);

    // Incoming adjacency: parents of each node, only edges between scene atoms.
    const parentsOf = new Map<string, string[]>();
    for (const id of nodeIds) parentsOf.set(id, []);
    const sortedRelations = Array.from(input.relations).sort((a, b) => {
      if (a.sourceId !== b.sourceId) return a.sourceId.localeCompare(b.sourceId);
      if (a.targetId !== b.targetId) return a.targetId.localeCompare(b.targetId);
      return a.id.localeCompare(b.id);
    });
    for (const relation of sortedRelations) {
      const parent = relation.sourceId;
      const child = relation.targetId;
      if (parent === child) continue;
      if (!nodeIdSet.has(parent) || !nodeIdSet.has(child)) continue;
      parentsOf.get(child)!.push(parent);
    }

    const layerOf = computeLayers(nodeIds, parentsOf);

    // Group node ids by layer, sorted within each layer for stable stacking.
    const byLayer = new Map<number, string[]>();
    for (const id of nodeIds) {
      const layer = layerOf.get(id) ?? 0;
      const bucket = byLayer.get(layer);
      if (bucket === undefined) byLayer.set(layer, [id]);
      else bucket.push(id);
    }

    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const [layer, members] of byLayer) {
      const x = layer * layerGap;
      members
        .slice()
        .sort((a, b) => a.localeCompare(b))
        .forEach((id, rowIndex) => {
          const y = rowIndex * nodeGap;
          positions.set(id, { x, y, space: 'diagram' });
          if (x < minX) minX = x;
          if (x > maxX) maxX = x;
          if (y < minY) minY = y;
          if (y > maxY) maxY = y;
        });
    }

    return {
      coordinateSpace: 'diagram',
      positions,
      cameraHint: { bounds: { minX, minY, maxX, maxY } },
    };
  },
  terminalState(input) {
    return genericTerminalState(input, 'flow_layered', 'diagram');
  },
};

/**
 * Longest-path layering. layer(n) = 0 when n has no parents, else
 * max(layer(parent) + 1) over parents. Memoized; a parent currently on the
 * recursion stack is a back-edge (cycle) and is skipped so it does not extend
 * depth. Visiting nodes and parents in sorted order makes the result
 * deterministic even when the graph has cycles.
 */
function computeLayers(
  nodeIds: readonly string[],
  parentsOf: Map<string, string[]>,
): Map<string, number> {
  const layer = new Map<string, number>();
  const onStack = new Set<string>();

  const visit = (id: string): number => {
    const cached = layer.get(id);
    if (cached !== undefined) return cached;
    if (onStack.has(id)) return 0; // back-edge: do not recurse into a cycle
    onStack.add(id);
    let max = 0;
    const parents = parentsOf.get(id) ?? [];
    for (const parent of [...parents].sort((a, b) => a.localeCompare(b))) {
      if (onStack.has(parent)) continue; // back-edge
      const candidate = visit(parent) + 1;
      if (candidate > max) max = candidate;
    }
    onStack.delete(id);
    layer.set(id, max);
    return max;
  };

  for (const id of nodeIds) visit(id);
  return layer;
}

function positiveOr(value: number | undefined, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : fallback;
}
