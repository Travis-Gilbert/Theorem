/**
 * v2 ProjectionCapability and ChromeCapability.
 *
 * v1 modeled scene types as a flat ``SceneRendererCapability`` (one
 * row per engine). v2 splits this into two typed peers:
 *
 *   * ProjectionCapability: placement function (atoms -> positions in
 *     one CoordinateSpace).
 *   * ChromeCapability: UI shell composed around the substrate.
 *
 * The trusted catalog imports these two types and validates them
 * against the registries; a missing or unknown id is a compile-time
 * refusal, not a runtime fallback.
 *
 * Existing v1 SceneRendererCapability continues to live in ../types.ts
 * so currently shipping recipes keep working. Stage 02+ wires the
 * compiler to emit a v2 package alongside v1.
 */

import type { CoordinateSpace } from './atoms/types';

export type ProjectionDrives = 'position' | 'scale' | 'color' | 'opacity' | 'glyph';

export type ProjectionInteraction =
  | 'select'
  | 'hover'
  | 'filter'
  | 'compare'
  | 'weight'
  | 'playback'
  | 'zoom'
  | 'annotate'
  | 'open-evidence'
  | 'save'
  | 'ask-follow-up';

export type ProjectionPatchSupport =
  | 'full-replace'
  | 'atom-update'
  | 'relation-update'
  | 'panel-update'
  | 'viewport-update'
  | 'state-update';

export interface ProjectionRequirements {
  atomFields?: readonly string[];
  relationFields?: readonly string[];
  minAtoms?: number;
  maxAtoms?: number;
  /** Optional canonical source shape (ranked_set / geo_set / network /
   *  timeline / comparison_matrix / ...). Carried so the compiler can
   *  reject scenes whose data shape doesn't match the projection's
   *  expectations. */
  sourceShape?: string;
}

export interface ProjectionAttributes {
  /** Which visual attributes the projection drives directly. Other
   *  attributes pass through from the atom or are driven by chrome. */
  drives: readonly ProjectionDrives[];
}

export interface ProjectionBudgets {
  maxAtoms?: number;
  maxRelations?: number;
  maxImages?: number;
  maxFrames?: number;
  maxPayloadBytes?: number;
  expectedFps?: number;
}

export interface ProjectionCapability {
  id: string;
  label: string;
  coordinateSpace: CoordinateSpace;
  requires?: ProjectionRequirements;
  attributes: ProjectionAttributes;
  interactions: readonly ProjectionInteraction[];
  patchSupport: readonly ProjectionPatchSupport[];
  budgets?: ProjectionBudgets;
  fallbackProjection?: string;
  emitsTerminalState: boolean;
}

export type ChromeAffordance =
  | 'player'
  | 'narration'
  | 'evidence-drawer'
  | 'compare-toolbar'
  | 'exploration-palette'
  | 'document-rail'
  | 'dashboard-grid'
  | 'gallery-rail';

export type ChromeScreenRegion = 'top' | 'bottom' | 'left' | 'right' | 'overlay';

export type ChromePatchSupport = 'full-replace' | 'state-update';

export interface ChromeCapability {
  id: string;
  label: string;
  affordances: readonly ChromeAffordance[];
  reservesScreenRegions: readonly ChromeScreenRegion[];
  pairsWithProjections: readonly string[];
  patchSupport: readonly ChromePatchSupport[];
}

// --------------------------------------------------------------------------
// Validation
// --------------------------------------------------------------------------

const PROJECTION_DRIVES: readonly ProjectionDrives[] = [
  'position',
  'scale',
  'color',
  'opacity',
  'glyph',
];

const INTERACTIONS: readonly ProjectionInteraction[] = [
  'select',
  'hover',
  'filter',
  'compare',
  'weight',
  'playback',
  'zoom',
  'annotate',
  'open-evidence',
  'save',
  'ask-follow-up',
];

const PATCH_SUPPORT: readonly ProjectionPatchSupport[] = [
  'full-replace',
  'atom-update',
  'relation-update',
  'panel-update',
  'viewport-update',
  'state-update',
];

const CHROME_AFFORDANCES: readonly ChromeAffordance[] = [
  'player',
  'narration',
  'evidence-drawer',
  'compare-toolbar',
  'exploration-palette',
  'document-rail',
  'dashboard-grid',
  'gallery-rail',
];

const CHROME_REGIONS: readonly ChromeScreenRegion[] = [
  'top',
  'bottom',
  'left',
  'right',
  'overlay',
];

const CHROME_PATCH_SUPPORT: readonly ChromePatchSupport[] = ['full-replace', 'state-update'];

export function validateProjectionCapability(cap: ProjectionCapability): string | null {
  if (!cap.id || !cap.label) {
    return 'ProjectionCapability requires id and label';
  }
  for (const drive of cap.attributes.drives) {
    if (!PROJECTION_DRIVES.includes(drive)) {
      return `ProjectionCapability ${cap.id}: drive ${drive} is not a known drive keyword`;
    }
  }
  for (const interaction of cap.interactions) {
    if (!INTERACTIONS.includes(interaction)) {
      return `ProjectionCapability ${cap.id}: interaction ${interaction} is not a known interaction keyword`;
    }
  }
  for (const patch of cap.patchSupport) {
    if (!PATCH_SUPPORT.includes(patch)) {
      return `ProjectionCapability ${cap.id}: patchSupport ${patch} is not a known patch keyword`;
    }
  }
  return null;
}

export function validateChromeCapability(cap: ChromeCapability): string | null {
  if (!cap.id || !cap.label) {
    return 'ChromeCapability requires id and label';
  }
  for (const affordance of cap.affordances) {
    if (!CHROME_AFFORDANCES.includes(affordance)) {
      return `ChromeCapability ${cap.id}: affordance ${affordance} is not a known affordance`;
    }
  }
  for (const region of cap.reservesScreenRegions) {
    if (!CHROME_REGIONS.includes(region)) {
      return `ChromeCapability ${cap.id}: reservesScreenRegions ${region} is not a known region`;
    }
  }
  for (const patch of cap.patchSupport) {
    if (!CHROME_PATCH_SUPPORT.includes(patch)) {
      return `ChromeCapability ${cap.id}: patchSupport ${patch} is not a known patch keyword`;
    }
  }
  return null;
}
