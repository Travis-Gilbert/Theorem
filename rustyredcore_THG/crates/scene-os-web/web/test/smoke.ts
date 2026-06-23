/**
 * Headless smoke for the pure scene pipeline (no DOM).
 *
 * Verifies that each of the bundle's eight production projection ids resolves
 * to an adapter, places every atom at a finite position, and produces non-
 * degenerate bounds, plus the two honesty paths (unknown id -> freeform
 * fallback, positionless atoms -> grid fallback). Bundled to CJS by esbuild and
 * run under node; the canvas drawing is verified separately by a browser
 * screenshot.
 */

import type { Atom, Relation } from '../src/atoms/types';
import type { ScenePackageV2 } from '../src/v2-package';
import { layoutScene } from '../src/renderer/sceneGeometry';
import { resolveProjection, supportedProjectionIds } from '../src/projections/productionRegistry';

let failures = 0;
function check(label: string, cond: boolean): void {
  if (cond) {
    console.log(`  ok   ${label}`);
  } else {
    console.error(`  FAIL ${label}`);
    failures += 1;
  }
}

function atom(id: string, kind: string, meta?: Record<string, unknown>): Atom {
  return { id, kind, label: `${kind} ${id}`, lifecycle: 'present', metadata: meta };
}
function relation(id: string, sourceId: string, targetId: string): Relation {
  return { id, sourceId, targetId, kind: 'supports', lifecycle: 'present' };
}

function pkg(projectionId: string, atoms: Atom[], relations: Relation[]): ScenePackageV2 {
  return {
    version: 'scene-package-v2',
    id: `pkg-${projectionId}`,
    manifestRef: 'm1',
    atoms,
    relations,
    projection: { id: projectionId },
    chrome: { id: 'document_rail' },
    actions: [],
  };
}

const viewport = { width: 1280, height: 720 };

// A small connected graph reused across projections.
const atoms: Atom[] = [
  atom('a', 'claim', { value: 5, order: 0, category: 'A' }),
  atom('b', 'evidence', { value: 9, order: 1, category: 'A' }),
  atom('c', 'source', { value: 3, order: 2, category: 'B' }),
  atom('d', 'concept', { value: 7, order: 3, category: 'B' }),
];
const relations: Relation[] = [
  relation('b-a', 'b', 'a'),
  relation('c-b', 'c', 'b'),
  relation('d-a', 'd', 'a'),
];

console.log('registry');
check('eight projection ids registered', supportedProjectionIds().length === 8);
for (const id of [
  'patent_diagram',
  'tree_hierarchy',
  'numeric_series',
  'categorical_set',
  'flow_layered',
  'sankey_flow',
  'graph_force',
  'geo',
]) {
  check(`resolves ${id} without fallback`, resolveProjection(id).fellBack === false);
}

for (const id of supportedProjectionIds()) {
  console.log(`projection ${id}`);
  const layout = layoutScene(pkg(id, atoms, relations), viewport);
  check(`${id}: not a fallback`, layout.fellBack === false);
  check(`${id}: places every atom`, layout.positions.size === atoms.length);
  let allFinite = true;
  for (const p of layout.positions.values()) {
    if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) allFinite = false;
  }
  check(`${id}: all positions finite`, allFinite);
  const spanX = layout.bounds.maxX - layout.bounds.minX;
  const spanY = layout.bounds.maxY - layout.bounds.minY;
  check(`${id}: non-degenerate bounds`, spanX > 0 || spanY > 0);
  check(`${id}: coordinate space set`, layout.coordinateSpace.length > 0);
}

console.log('fallbacks');
const unknown = layoutScene(pkg('does_not_exist', atoms, relations), viewport);
check('unknown id falls back to freeform', unknown.fellBack === true);
check('fallback still places every atom', unknown.positions.size === atoms.length);

// Positionless atoms under freeform -> grid fallback (visible, not one dot).
const bare = [atom('x', 'note'), atom('y', 'note'), atom('z', 'note')];
const grid = layoutScene(pkg('freeform_missing', bare, []), viewport);
check('degenerate placement triggers grid fallback', grid.gridFallback === true);
const xs = new Set(Array.from(grid.positions.values()).map((p) => `${p.x},${p.y}`));
check('grid fallback spreads atoms to distinct cells', xs.size === bare.length);

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall scene-pipeline checks passed');
