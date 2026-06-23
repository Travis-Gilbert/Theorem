/**
 * Tree-hierarchy projection adapter.
 *
 * Pure placement function. Wraps d3-hierarchy's `tree()` tidy-tree layout
 * (Reingold-Tilford via d3) as a Scene OS projection: it turns the atom +
 * relation set into a rooted forest and lays it out in the `diagram`
 * coordinate space, so any parent-child structure the substrate knows
 * (taxonomies, file trees, claim-support chains, org structures) renders as
 * a tidy tree rather than a force hairball.
 *
 * The hard part is that substrate relations are an arbitrary directed graph,
 * but `d3.hierarchy` requires a tree (each node one parent, no cycles, single
 * root). This adapter coerces the graph into a forest deterministically:
 *
 *   1. Inputs are sorted (atoms by id, relations by source/target/id) so the
 *      layout is reproducible regardless of input order.
 *   2. Each atom is assigned its FIRST-seen parent in sorted relation order;
 *      extra parents are ignored (a graph node with two parents becomes a
 *      tree node under the lexicographically-first relation's source).
 *   3. A candidate parent that would close a cycle (the proposed parent is a
 *      descendant of the child) is skipped, so cycles cannot wedge the layout.
 *   4. Atoms with no surviving parent are roots; all roots hang under a
 *      synthetic super-root so a forest lays out as one tidy tree. The
 *      synthetic root itself is never emitted as a position.
 *
 * Every input atom receives a position (the contract requires it). Isolated
 * atoms are roots and lay out alongside the real roots.
 *
 * Determinism: sorted inputs + d3's deterministic tidy-tree + `nodeSize`
 * (spacing independent of node count) make `project` and `terminalState`
 * byte-reproducible, satisfying the projection contract's purity requirement.
 */

import { hierarchy, tree, type HierarchyNode } from 'd3-hierarchy';

import type { AtomPosition } from '../atoms/types';
import type {
  ProjectionAdapter,
  ProjectionInput,
  ProjectionOutput,
} from '../substrate/projection';
import { genericTerminalState } from '../substrate/projection';

export interface TreeHierarchyProjectionParams {
  /** 'vertical' lays roots at the top, depth increasing downward (default).
   *  'horizontal' lays roots at the left, depth increasing rightward. */
  orientation?: 'vertical' | 'horizontal';
  /** Sibling separation in substrate units (the d3.tree nodeSize cross-axis).
   *  Default 48. */
  siblingGap?: number;
  /** Depth separation in substrate units (the d3.tree nodeSize depth-axis).
   *  Default 96. */
  depthGap?: number;
}

const SYNTHETIC_ROOT_ID = '__tree_root__';

interface TreeNodeData {
  id: string;
  children: TreeNodeData[];
}

export const TREE_HIERARCHY_PROJECTION: ProjectionAdapter = {
  id: 'tree_hierarchy',
  label: 'Tree Hierarchy',
  coordinateSpace: 'diagram',
  hostOverlay: 'none',
  supportedAtomKinds: undefined,
  project(input: ProjectionInput): ProjectionOutput {
    const params = (input.host ?? {}) as TreeHierarchyProjectionParams;
    const orientation = params.orientation === 'horizontal' ? 'horizontal' : 'vertical';
    const siblingGap = positiveOr(params.siblingGap, 48);
    const depthGap = positiveOr(params.depthGap, 96);

    const positions = new Map<string, AtomPosition>();
    if (input.atoms.length === 0) {
      return { coordinateSpace: 'diagram', positions };
    }

    // 1. Deterministic ordering.
    const atomIds = Array.from(input.atoms, (atom) => atom.id).sort((a, b) =>
      a.localeCompare(b),
    );
    const atomIdSet = new Set(atomIds);
    const sortedRelations = Array.from(input.relations).sort((a, b) => {
      if (a.sourceId !== b.sourceId) return a.sourceId.localeCompare(b.sourceId);
      if (a.targetId !== b.targetId) return a.targetId.localeCompare(b.targetId);
      return a.id.localeCompare(b.id);
    });

    // 2 + 3. First-parent assignment with cycle breaking. Only relations whose
    // BOTH endpoints are atoms in this scene participate.
    const parentOf = new Map<string, string>();
    for (const relation of sortedRelations) {
      const parent = relation.sourceId;
      const child = relation.targetId;
      if (!atomIdSet.has(parent) || !atomIdSet.has(child)) continue;
      if (parent === child) continue; // self-loop
      if (parentOf.has(child)) continue; // keep first-seen parent
      if (wouldCycle(parentOf, parent, child)) continue; // parent is below child
      parentOf.set(child, parent);
    }

    // 4. children map + roots, both in sorted order.
    const childrenOf = new Map<string, string[]>();
    for (const id of atomIds) childrenOf.set(id, []);
    const roots: string[] = [];
    for (const id of atomIds) {
      const parent = parentOf.get(id);
      if (parent === undefined) {
        roots.push(id);
      } else {
        childrenOf.get(parent)!.push(id);
      }
    }

    const buildNode = (id: string): TreeNodeData => ({
      id,
      children: childrenOf.get(id)!.map(buildNode),
    });
    const rootData: TreeNodeData = {
      id: SYNTHETIC_ROOT_ID,
      children: roots.map(buildNode),
    };

    const root = hierarchy<TreeNodeData>(rootData, (d) => d.children);
    // nodeSize keeps spacing constant regardless of tree size; the substrate
    // camera fits to the resulting bounds via cameraHint.
    const layout = tree<TreeNodeData>().nodeSize([siblingGap, depthGap]);
    layout(root);

    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    root.each((node: HierarchyNode<TreeNodeData> & { x?: number; y?: number }) => {
      if (node.data.id === SYNTHETIC_ROOT_ID) return;
      // d3.tree assigns node.x across siblings and node.y by depth. For a
      // vertical tree that maps directly; for horizontal we swap the axes.
      const tx = node.x ?? 0;
      const ty = node.y ?? 0;
      const x = orientation === 'horizontal' ? ty : tx;
      const y = orientation === 'horizontal' ? tx : ty;
      positions.set(node.data.id, { x, y, space: 'diagram' });
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (y < minY) minY = y;
      if (y > maxY) maxY = y;
    });

    const cameraHint =
      positions.size > 0 && Number.isFinite(minX)
        ? { bounds: { minX, minY, maxX, maxY } }
        : undefined;

    return { coordinateSpace: 'diagram', positions, cameraHint };
  },
  terminalState(input) {
    return genericTerminalState(input, 'tree_hierarchy', 'diagram');
  },
};

/** Returns true if making `parent` the parent of `child` would create a
 *  cycle, i.e. `parent` is already a descendant of `child` along the
 *  first-parent chain. Walks up from `parent`; if it reaches `child`, the
 *  edge closes a loop. */
function wouldCycle(
  parentOf: Map<string, string>,
  parent: string,
  child: string,
): boolean {
  let cursor: string | undefined = parent;
  const guard = new Set<string>();
  while (cursor !== undefined) {
    if (cursor === child) return true;
    if (guard.has(cursor)) return true; // defensive: pre-existing loop
    guard.add(cursor);
    cursor = parentOf.get(cursor);
  }
  return false;
}

function positiveOr(value: number | undefined, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : fallback;
}
