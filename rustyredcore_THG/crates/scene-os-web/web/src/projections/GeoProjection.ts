/**
 * Geographic projection adapter (the map layout). Proves d3-geo generalizes
 * the renderer into the `geo` coordinate space.
 *
 * Pure placement function. Reads each atom's longitude/latitude from its
 * metadata, builds a deterministic d3-geo Mercator that fits every coordinate
 * into a virtual layout box, and projects each lng/lat to an (x, y) the
 * substrate camera then fits to the canvas. This is the projection for
 * "where on the map" query shapes (places, regions, routes, anything carrying
 * geographic coordinates), the same way `numeric_series` is the projection for
 * "over a numeric axis" shapes.
 *
 * Coordinate resolution per atom (first present pair wins, case-insensitive on
 * the metadata key): lat/lng, latitude/longitude, lat/lon. A value may be a
 * number or a numeric string. Atoms with no resolvable coordinate are NOT
 * dropped (the contract requires a position for every atom): they are placed in
 * a deterministic left-margin column, ordered by sorted id, so an atom that
 * lacks geography still appears and stays stable across runs (never a random
 * draw).
 *
 * Determinism: the projection is fit to the sorted set of coordinates via
 * `fitExtent`, d3-geo's Mercator is a pure function, and the fallback column is
 * keyed by sorted-id index. So `project` and `terminalState` are byte-
 * reproducible, satisfying the projection contract's purity requirement.
 */

import { geoMercator } from 'd3-geo';

import type { AtomPosition } from '../atoms/types';
import type {
  ProjectionAdapter,
  ProjectionInput,
  ProjectionOutput,
} from '../substrate/projection';
import { genericTerminalState } from '../substrate/projection';

export interface GeoProjectionParams {
  /** Metadata key holding each atom's longitude. When absent, the resolver
   *  tries the standard key pairs (lng / longitude / lon). */
  lngKey?: string;
  /** Metadata key holding each atom's latitude. When absent, the resolver
   *  tries the standard key pairs (lat / latitude). */
  latKey?: string;
  /** Inner padding inside the virtual layout box so coastal points are not
   *  flush to the edge. Default 48. */
  padding?: number;
}

/** Virtual layout space, matching GraphForceProjection so absolute tuning
 *  (padding, fallback column) stays stable across viewport sizes; the renderer
 *  fits this box to the canvas. */
const VW = 1080;
const VH = 680;

/** Candidate (lng, lat) metadata key pairs, tried in order after any explicit
 *  params. Lowercased before lookup so `Lat`/`LATITUDE` resolve too. */
const COORD_KEY_PAIRS: ReadonlyArray<readonly [string, string]> = [
  ['lng', 'lat'],
  ['longitude', 'latitude'],
  ['lon', 'lat'],
];

interface GeoCoord {
  id: string;
  lng: number;
  lat: number;
}

export const GEO_PROJECTION: ProjectionAdapter = {
  id: 'geo',
  label: 'Geo',
  coordinateSpace: 'geo',
  hostOverlay: 'none',
  supportedAtomKinds: undefined,
  project(input: ProjectionInput): ProjectionOutput {
    const params = (input.host ?? {}) as GeoProjectionParams;
    const padding = positiveOr(params.padding, 48);
    const positions = new Map<string, AtomPosition>();
    if (input.atoms.length === 0) {
      return { coordinateSpace: 'geo', positions };
    }

    // Deterministic id order drives both the fallback column index and the
    // order coordinates feed the projection's fit.
    const sortedAtoms = Array.from(input.atoms).sort((a, b) =>
      a.id.localeCompare(b.id),
    );

    // Resolve coordinates; partition into geo-positioned and coordinate-less.
    const coords: GeoCoord[] = [];
    const withoutCoords: string[] = [];
    for (const atom of sortedAtoms) {
      const meta = lowerCaseKeys(atom.metadata ?? {});
      const resolved = resolveCoord(meta, params.lngKey, params.latKey);
      if (resolved === null) {
        withoutCoords.push(atom.id);
      } else {
        coords.push({ id: atom.id, lng: resolved.lng, lat: resolved.lat });
      }
    }

    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    const note = (x: number, y: number): void => {
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (y < minY) minY = y;
      if (y > maxY) maxY = y;
    };

    if (coords.length > 0) {
      // Fit a Mercator to every coordinate so the whole set sits inside the
      // padded virtual box. fitExtent is deterministic for a fixed point set.
      const projection = geoMercator().fitExtent(
        [
          [padding, padding],
          [VW - padding, VH - padding],
        ],
        {
          type: 'MultiPoint',
          coordinates: coords.map((c) => [c.lng, c.lat]),
        },
      );
      for (const coord of coords) {
        const projected = projection([coord.lng, coord.lat]);
        // Mercator returns null for points outside its clip bounds; fall back
        // to the box center so the atom is still placed (contract: every atom
        // gets a position) rather than dropped.
        const x = projected ? projected[0] : VW / 2;
        const y = projected ? projected[1] : VH / 2;
        positions.set(coord.id, { x, y, space: 'geo' });
        note(x, y);
      }
    }

    // Coordinate-less atoms occupy a deterministic left-margin column so they
    // remain visible and stable (never a random scatter). They sit just left of
    // the fit box, spaced down the virtual height.
    if (withoutCoords.length > 0) {
      const colX = Math.max(8, padding * 0.4);
      const usableH = VH - padding * 2;
      const step =
        withoutCoords.length > 1 ? usableH / (withoutCoords.length - 1) : 0;
      withoutCoords.forEach((id, index) => {
        const x = colX;
        const y = padding + (withoutCoords.length > 1 ? step * index : usableH / 2);
        positions.set(id, { x, y, space: 'geo' });
        note(x, y);
      });
    }

    return {
      coordinateSpace: 'geo',
      positions,
      cameraHint: Number.isFinite(minX) ? { bounds: { minX, minY, maxX, maxY } } : undefined,
    };
  },
  terminalState(input) {
    return genericTerminalState(input, 'geo', 'geo');
  },
};

/** Lowercase every top-level metadata key so coordinate lookup is case-
 *  insensitive (handles `Lat`, `LONGITUDE`, etc.). Later duplicate keys (after
 *  lowercasing) overwrite earlier ones, which is acceptable for coordinates. */
function lowerCaseKeys(meta: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(meta)) {
    out[key.toLowerCase()] = meta[key];
  }
  return out;
}

/** Resolve a (lng, lat) pair from lowercased metadata. Explicit param keys are
 *  tried first (also lowercased), then the standard key pairs. Returns null
 *  when no complete pair resolves to finite numbers. */
function resolveCoord(
  meta: Record<string, unknown>,
  lngKey: string | undefined,
  latKey: string | undefined,
): { lng: number; lat: number } | null {
  if (lngKey !== undefined && latKey !== undefined) {
    const lng = toNumber(meta[lngKey.toLowerCase()]);
    const lat = toNumber(meta[latKey.toLowerCase()]);
    if (lng !== null && lat !== null) return { lng, lat };
  }
  for (const [lk, ltk] of COORD_KEY_PAIRS) {
    const lng = toNumber(meta[lk]);
    const lat = toNumber(meta[ltk]);
    if (lng !== null && lat !== null) return { lng, lat };
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

function positiveOr(value: number | undefined, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : fallback;
}
