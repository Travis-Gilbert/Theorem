/**
 * Patent diagram projection adapter.
 *
 * Unlike the canvas-rendered projections (geo, graph, matrix, cinematic,
 * image), patent_diagram is *artifact-shaped*: the agent has already
 * produced a fully-laid-out PatentScenePayload (DOT source, callouts,
 * legend, title block). The substrate's canvas is therefore optional :
 * the chrome shell (`patent_plate_shell`) renders the patent via the
 * existing `<PatentDiagram>` primitive, which runs Graphviz layout
 * inside its own React tree.
 *
 * This adapter's role is reduced:
 *  - project(): returns simple top-to-bottom column positions so the
 *    substrate has SOMETHING to keyframe / morph between. The
 *    canonical layout still happens inside <PatentDiagram> via wasm
 *    Graphviz.
 *  - terminalState(): the BACKEND resolver already populated
 *    package.terminal_state with the full PatentScenePayload as JSON,
 *    so this adapter just returns the package's existing
 *    terminal_state slot. The artifact persistence endpoint can
 *    serialize JSON → SVG on save by re-rendering <PatentDiagram>
 *    server-side if needed.
 *
 * Stable atom IDs (`f<figure_number>.<dot_node_id>`) come from the
 * resolver upstream so the choreographer can morph between recompiled
 * patent scenes.
 */

import type { Atom, AtomPosition } from '../atoms/types';
import {
  genericTerminalState,
  type ProjectionAdapter,
  type ProjectionInput,
  type ProjectionOutput,
} from '../substrate/projection';

export interface PatentDiagramProjectionParams {
  /** Pass-through of the agent-emitted PatentScenePayload. The chrome
   *  shell reads this from `package.projection.params.payload` and
   *  hands it to the existing <PatentDiagram> primitive. The adapter
   *  itself doesn't render it; canvas positions are computed below
   *  purely so the substrate's choreographer has placement data. */
  payload?: Record<string, unknown>;
  /** When true, the cascade fell back to 4B; chrome surfaces the
   *  caveat strip. */
  degraded?: boolean;
  /** '26b' | '4b' | etc. */
  model_used?: string;
  /** Horizontal spacing between figure columns in canvas coords. */
  figureSpacing?: number;
  /** Vertical spacing between atoms in the same figure. */
  atomSpacing?: number;
}

/**
 * Positions atoms in a simple column-per-figure layout. The canonical
 * patent layout (Graphviz dot) happens inside <PatentDiagram>; these
 * positions only matter for substrate-level interactions (selection,
 * hover, morph keyframes).
 */
export function layoutPatentAtoms(
  atoms: readonly Atom[],
  params: PatentDiagramProjectionParams,
): Map<string, AtomPosition> {
  const figureSpacing = params.figureSpacing ?? 480;
  const atomSpacing = params.atomSpacing ?? 80;

  // Group atoms by figure_number (metadata key set by the backend
  // resolver). Atoms without a figure_number land in figure 0.
  const byFigure = new Map<number, Atom[]>();
  for (const atom of atoms) {
    const figRaw = atom.metadata?.figure_number;
    const fig = typeof figRaw === 'number' ? figRaw : 0;
    if (!byFigure.has(fig)) byFigure.set(fig, []);
    byFigure.get(fig)!.push(atom);
  }

  const positions = new Map<string, AtomPosition>();
  const sortedFigures = Array.from(byFigure.keys()).sort((a, b) => a - b);

  for (let figIdx = 0; figIdx < sortedFigures.length; figIdx += 1) {
    const figNum = sortedFigures[figIdx];
    const figAtoms = byFigure.get(figNum)!;
    const x = figIdx * figureSpacing;
    for (let i = 0; i < figAtoms.length; i += 1) {
      const atom = figAtoms[i];
      const y = i * atomSpacing - (figAtoms.length * atomSpacing) / 2;
      positions.set(atom.id, {
        x,
        y,
        z: 0,
        space: 'diagram',
      });
    }
  }
  return positions;
}

export const PATENT_DIAGRAM_PROJECTION: ProjectionAdapter = {
  id: 'patent_diagram',
  label: 'Patent Diagram',
  coordinateSpace: 'diagram',
  // Patent_plate_shell renders the patent via <PatentDiagram>'s own
  // SVG tree, not via a host overlay over the canvas. The host overlay
  // declaration would imply a separate layer; here the chrome shell IS
  // the visual layer.
  hostOverlay: undefined,
  supportedAtomKinds: ['patent-node'],
  project(input: ProjectionInput): ProjectionOutput {
    const { atoms } = input;
    const params = (input.host ?? {}) as PatentDiagramProjectionParams;
    const positions = layoutPatentAtoms(atoms, params);

    // Camera hint: rectangle covering all positions so substrate can
    // fit them on first paint. The chrome doesn't actually need this
    // (it doesn't show the substrate canvas) but the substrate's
    // choreographer respects it during morph transitions in/out.
    let minX = Number.POSITIVE_INFINITY;
    let minY = Number.POSITIVE_INFINITY;
    let maxX = Number.NEGATIVE_INFINITY;
    let maxY = Number.NEGATIVE_INFINITY;
    for (const pos of positions.values()) {
      if (pos.x < minX) minX = pos.x;
      if (pos.y < minY) minY = pos.y;
      if (pos.x > maxX) maxX = pos.x;
      if (pos.y > maxY) maxY = pos.y;
    }

    return {
      coordinateSpace: 'diagram',
      positions,
      hostOverlay: undefined,
      cameraHint: Number.isFinite(minX)
        ? { bounds: { minX, minY, maxX, maxY } }
        : undefined,
    };
  },
  terminalState(input) {
    // Delegate to the generic SVG generator for the substrate-level
    // SVG artifact (deterministic, sortable, source-refs aggregated).
    // The full patent_scene payload is in package.projection.params :
    // not on TerminalStateInput: so the chrome's freeze handler reads
    // it from the package directly and POSTs a richer artifact when
    // needed. The substrate-level SVG here serves as the always-
    // available baseline artifact (per the v2 spec's determinism
    // rule).
    return genericTerminalState(input, 'patent_diagram', 'diagram');
  },
};
