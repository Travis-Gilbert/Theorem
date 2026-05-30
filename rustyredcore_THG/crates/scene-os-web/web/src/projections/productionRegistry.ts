/**
 * Production projection registry for the Theorem renderer bundle.
 *
 * Lane A (scene-os-core, Rust) emits exactly six projection ids in its
 * production catalog (`production_projection_catalog`):
 *
 *   patent_diagram, tree_hierarchy, numeric_series,
 *   categorical_set, flow_layered, sankey_flow
 *
 * This bundle eagerly imports the six matching adapters and resolves an id to
 * its adapter. Unlike the Theseus-UI app's lazy `registry.ts` (dynamic-import
 * per projection so the SPA stays trim), the browser bundle ships a single
 * self-contained asset, so eager imports are correct: there is no second
 * chunk to defer to, and Servo serves one file.
 *
 * `resolveProjection` never throws: an unknown id falls back to
 * FREEFORM_PROJECTION (the same safety net the substrate contract defines),
 * so a director that emits a not-yet-ported projection id renders the atoms in
 * freeform space rather than a blank canvas. The fallback is reported through
 * the returned `{ adapter, fellBack }` so the renderer can surface an honest
 * "rendered in freeform (projection X not available)" note rather than lie.
 */

import { FREEFORM_PROJECTION } from '../substrate/projection';
import type { ProjectionAdapter } from '../substrate/projection';

import { PATENT_DIAGRAM_PROJECTION } from './PatentDiagramProjection';
import { TREE_HIERARCHY_PROJECTION } from './TreeHierarchyProjection';
import { NUMERIC_SERIES_PROJECTION } from './NumericSeriesProjection';
import { CATEGORICAL_SET_PROJECTION } from './CategoricalSetProjection';
import { FLOW_LAYERED_PROJECTION } from './FlowLayeredProjection';
import { SANKEY_FLOW_PROJECTION } from './SankeyFlowProjection';
import { GRAPH_FORCE_PROJECTION } from './GraphForceProjection';

/**
 * The adapters this bundle can render, keyed by id.
 *
 * The first six are Lane A's production catalog set. `graph_force` is the
 * renderer-side d3-force layout: it runs the simulation synchronously and
 * returns settled positions (it does NOT require cosmos.gl, unlike the
 * Theseus-UI graph adapter), so the canvas draws a well-spaced constellation
 * for any graph-shaped scene — the layout the coordination room needs.
 */
const PRODUCTION_PROJECTIONS: ReadonlyArray<ProjectionAdapter> = [
  PATENT_DIAGRAM_PROJECTION,
  TREE_HIERARCHY_PROJECTION,
  NUMERIC_SERIES_PROJECTION,
  CATEGORICAL_SET_PROJECTION,
  FLOW_LAYERED_PROJECTION,
  SANKEY_FLOW_PROJECTION,
  GRAPH_FORCE_PROJECTION,
];

const REGISTRY: ReadonlyMap<string, ProjectionAdapter> = new Map(
  PRODUCTION_PROJECTIONS.map((adapter) => [adapter.id, adapter]),
);

export interface ResolvedProjection {
  adapter: ProjectionAdapter;
  /** True when the requested id was not in the registry and the freeform
   *  safety net was substituted. The renderer surfaces this honestly. */
  fellBack: boolean;
  /** The id that was requested (so the renderer can name it in the note). */
  requestedId: string;
}

/** Resolve a projection id to its adapter, falling back to freeform for
 *  unknown ids. Never throws. */
export function resolveProjection(projectionId: string): ResolvedProjection {
  const adapter = REGISTRY.get(projectionId);
  if (adapter !== undefined) {
    return { adapter, fellBack: false, requestedId: projectionId };
  }
  return { adapter: FREEFORM_PROJECTION, fellBack: true, requestedId: projectionId };
}

/** The ids this bundle can render natively (the Lane A production set). */
export function supportedProjectionIds(): string[] {
  return Array.from(REGISTRY.keys());
}

export { PRODUCTION_PROJECTIONS };
