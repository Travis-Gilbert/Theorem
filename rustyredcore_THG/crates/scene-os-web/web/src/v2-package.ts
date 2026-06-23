/**
 * v2 ScenePackage schema. Mirrors apps/notebook/scene_os/v2_package.py.
 *
 * The v1 ``ScenePackage`` (../types.ts) continues to back currently
 * shipping recipes (it carries datasets, assets, traces, and a v1
 * manifest). v2 reframes the package around the substrate-and-projection
 * model: atoms, relations, projection, chrome, transitions, terminal
 * state.
 *
 * ``projection.id`` and ``chrome.id`` reference the trusted catalogs
 * defined in ``./capabilities.ts``. Validation rejects unknown ids,
 * missing required atom fields, oversized payloads, unknown action
 * ids, and patches that target absent atoms.
 */

import type { Atom, Relation } from './atoms/types';

export const SCENE_PACKAGE_V2_VERSION = 'scene-package-v2';

export interface ProjectionBinding {
  id: string;
  params?: Record<string, unknown>;
}

export interface ChromeBinding {
  id: string;
  params?: Record<string, unknown>;
}

export type ChoreographyMode = 'morph' | 'cut' | 'crossfade';

export interface TransitionDescriptor {
  /** ScenePackage id we morph FROM, or null on first paint. */
  from?: string;
  choreography: ChoreographyMode;
}

export interface TerminalStateArtifact {
  /** Rasterizable wire form. Either svg or json must be present. */
  svg?: string;
  /** Lossless re-mountable form. */
  json?: Record<string, unknown>;
  sourceRefs?: Array<Record<string, unknown>>;
}

export interface ActionDescriptor {
  id: string;
  label: string;
  actionType: string;
  /** Interaction keyword from the projection's
   *  ``ProjectionCapability.interactions``. The compiler refuses
   *  actions the projection doesn't support. */
  interaction: string;
  target?: string;
  payload?: Record<string, unknown>;
  requiresConfirmation?: boolean;
  proposalOnly?: boolean;
}

export interface ScenePackageV2 {
  version: typeof SCENE_PACKAGE_V2_VERSION;
  id: string;
  manifestRef: string;
  atoms: Atom[];
  relations: Relation[];
  projection: ProjectionBinding;
  chrome: ChromeBinding;
  actions: ActionDescriptor[];
  transitions?: TransitionDescriptor;
  terminalState?: TerminalStateArtifact;
  provenance?: Record<string, unknown>;
}

const CHOREOGRAPHY_MODES: readonly ChoreographyMode[] = ['morph', 'cut', 'crossfade'];

export function validateScenePackageV2(pkg: ScenePackageV2): string | null {
  if (pkg.version !== SCENE_PACKAGE_V2_VERSION) {
    return `ScenePackageV2 version mismatch: ${pkg.version}`;
  }
  if (!pkg.id || !pkg.manifestRef) {
    return 'ScenePackageV2 requires id and manifestRef';
  }
  if (!pkg.projection?.id) {
    return 'ScenePackageV2 requires projection.id';
  }
  if (!pkg.chrome?.id) {
    return 'ScenePackageV2 requires chrome.id';
  }
  if (pkg.transitions && !CHOREOGRAPHY_MODES.includes(pkg.transitions.choreography)) {
    return `ScenePackageV2 transitions.choreography ${pkg.transitions.choreography} is not valid`;
  }
  if (pkg.terminalState) {
    if (!pkg.terminalState.svg && !pkg.terminalState.json) {
      return 'ScenePackageV2 terminalState requires at least one of svg or json';
    }
  }
  // Action ids must be unique.
  const seen = new Set<string>();
  for (const action of pkg.actions) {
    if (seen.has(action.id)) {
      return `ScenePackageV2 duplicate action id ${action.id}`;
    }
    seen.add(action.id);
  }
  return null;
}
