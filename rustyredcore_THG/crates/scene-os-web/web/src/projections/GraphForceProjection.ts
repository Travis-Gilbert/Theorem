/**
 * Force-directed graph projection adapter (the well-spaced constellation).
 *
 * This is the d3-force layout the browser renderer was missing. Unlike the
 * Theseus-UI `graph_force` adapter (which only seeds atoms on a ring and
 * delegates the actual solve to cosmos.gl inside the React substrate), this
 * adapter runs the simulation ITSELF, synchronously, and returns settled
 * positions: so the vanilla canvas bundle draws a balanced constellation with
 * no cosmos.gl, no React, no live animation loop (the static result is painted
 * immediately, which is rAF-safe).
 *
 * The recipe is lifted from `renderers/graphLayout.ts` + the coordination-room
 * components, because the spacing is NOT "use a force graph": a naive force
 * graph is a hairball. The spacing comes from a specific composition:
 *
 *   charge:  forceManyBody, strong negative   (push apart)
 *   link:    long distance, weight-scaled      (breathing room on edges)
 *   collide: radius + padding                  (hard no-overlap)
 *   x / y:   forceX/forceY toward ZONE anchors (the anti-hairball trick:
 *            nodes settle into deliberate regions, not one blob)
 *   center:  forceCenter                        (keep it framed)
 *
 * Each atom's zone comes from `host.groups[id]` (exact control, e.g. a
 * coordination room placing agents left/right) or, failing that,
 * `metadata.group` / `metadata.community` / `kind` (auto-clustering). Named
 * zones (center/left/right/top/bottom/...) map to fixed regions; unknown groups
 * spread deterministically around an ellipse.
 *
 * Determinism: atoms seed at their zone anchor plus a per-index offset (no
 * Math.random), so the same scene settles to the same layout every run.
 */

import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from 'd3-force';

import type { Atom, AtomPosition } from '../atoms/types';
import type {
  ProjectionAdapter,
  ProjectionInput,
  ProjectionOutput,
} from '../substrate/projection';
import { genericTerminalState } from '../substrate/projection';

export interface GraphForceProjectionParams {
  /** Exact zone per atom id, overriding the metadata/kind fallback. Values are
   *  named zones (center/left/right/top/bottom/top-left/top-right/...) or any
   *  string (unknown strings spread around an ellipse). */
  groups?: Record<string, string>;
  /** Charge strength (push-apart). Default -820, the reference's value. */
  charge?: number;
  /** Base link distance in virtual units. Default 210. */
  linkDistance?: number;
  /** Iteration count for the synchronous settle. Default 260. */
  ticks?: number;
}

/** Virtual layout space. The renderer fits this to the canvas, so absolute
 *  tuning (charge, distance, radius) stays stable across viewport sizes: the
 *  same space the coordination-room reference tunes against. */
const VW = 1080;
const VH = 680;

/** Named zones as viewport fractions (mirrors the reference's groupTarget). */
const NAMED_ZONES: Readonly<Record<string, { x: number; y: number }>> = {
  center: { x: 0.5, y: 0.48 },
  left: { x: 0.18, y: 0.42 },
  right: { x: 0.82, y: 0.42 },
  top: { x: 0.5, y: 0.12 },
  bottom: { x: 0.5, y: 0.84 },
  'top-left': { x: 0.2, y: 0.18 },
  'top-right': { x: 0.8, y: 0.18 },
  'bottom-left': { x: 0.2, y: 0.82 },
  'bottom-right': { x: 0.8, y: 0.82 },
};

interface ForceNode extends SimulationNodeDatum {
  id: string;
  radius: number;
  homeX: number;
  homeY: number;
}

interface ForceLink extends SimulationLinkDatum<ForceNode> {
  source: string | ForceNode;
  target: string | ForceNode;
  strength: number;
}

export const GRAPH_FORCE_PROJECTION: ProjectionAdapter = {
  id: 'graph_force',
  label: 'Graph Force',
  coordinateSpace: 'graph',
  hostOverlay: 'none',
  supportedAtomKinds: undefined,
  project(input: ProjectionInput): ProjectionOutput {
    const params = (input.host ?? {}) as GraphForceProjectionParams;
    const positions = new Map<string, AtomPosition>();
    if (input.atoms.length === 0) {
      return { coordinateSpace: 'graph', positions };
    }

    const charge = typeof params.charge === 'number' ? params.charge : -820;
    const linkDistance = positiveOr(params.linkDistance, 210);
    const ticks = Math.max(1, Math.floor(positiveOr(params.ticks, 260)));

    const ids = input.atoms.map((atom) => atom.id);
    const idSet = new Set(ids);

    // Degree → centrality → radius (hubs read larger, matching the reference).
    const degree = new Map<string, number>(ids.map((id) => [id, 0]));
    const links: ForceLink[] = [];
    for (const rel of input.relations) {
      if (rel.sourceId === rel.targetId) continue;
      if (!idSet.has(rel.sourceId) || !idSet.has(rel.targetId)) continue;
      degree.set(rel.sourceId, (degree.get(rel.sourceId) ?? 0) + 1);
      degree.set(rel.targetId, (degree.get(rel.targetId) ?? 0) + 1);
      links.push({
        source: rel.sourceId,
        target: rel.targetId,
        strength: clamp01(typeof rel.weight === 'number' ? rel.weight : 0.5),
      });
    }
    const maxDegree = Math.max(1, ...Array.from(degree.values()));

    // Resolve a zone anchor per atom.
    const anchorByGroup = buildGroupAnchors(input.atoms, params.groups);
    const radiusById = new Map<string, number>();

    const nodes: ForceNode[] = input.atoms.map((atom, index) => {
      const centrality = (degree.get(atom.id) ?? 0) / maxDegree;
      const weighted =
        typeof atom.weight === 'number' && Number.isFinite(atom.weight)
          ? clamp01(atom.weight)
          : centrality;
      const radius = 18 + weighted * 20;
      radiusById.set(atom.id, radius);
      const group = resolveGroup(atom, params.groups);
      const anchor = anchorByGroup.get(group) ?? { x: VW / 2, y: VH / 2 };
      // Deterministic seed near the zone (no Math.random → reproducible).
      const angle = (index / input.atoms.length) * Math.PI * 2;
      return {
        id: atom.id,
        radius,
        homeX: anchor.x,
        homeY: anchor.y,
        x: anchor.x + Math.cos(angle) * 40,
        y: anchor.y + Math.sin(angle) * 40,
      };
    });

    const simulation = forceSimulation<ForceNode>(nodes)
      .force(
        'link',
        forceLink<ForceNode, ForceLink>(links)
          .id((node) => node.id)
          .distance((link) => linkDistance - link.strength * 40)
          .strength((link) => 0.1 + link.strength * 0.3),
      )
      .force('charge', forceManyBody<ForceNode>().strength(charge))
      .force('center', forceCenter(VW / 2, VH / 2))
      .force('x', forceX<ForceNode>((node) => node.homeX).strength(0.16))
      .force('y', forceY<ForceNode>((node) => node.homeY).strength(0.16))
      .force('collide', forceCollide<ForceNode>().radius((node) => node.radius + 18).iterations(2))
      .stop();

    for (let tick = 0; tick < ticks; tick += 1) simulation.tick();

    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const node of nodes) {
      const x = node.x ?? VW / 2;
      const y = node.y ?? VH / 2;
      positions.set(node.id, { x, y, space: 'graph' });
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (y < minY) minY = y;
      if (y > maxY) maxY = y;
    }

    return {
      coordinateSpace: 'graph',
      positions,
      cameraHint: Number.isFinite(minX) ? { bounds: { minX, minY, maxX, maxY } } : undefined,
    };
  },
  terminalState(input) {
    return genericTerminalState(input, 'graph_force', 'graph');
  },
};

/** Resolve the zone group for an atom: explicit override, then metadata, then
 *  kind. */
function resolveGroup(atom: Atom, groups: Record<string, string> | undefined): string {
  const override = groups?.[atom.id];
  if (typeof override === 'string' && override.length > 0) return override;
  const meta = atom.metadata ?? {};
  const fromMeta = meta.group ?? meta.community;
  if (typeof fromMeta === 'string' && fromMeta.length > 0) return fromMeta;
  if (typeof atom.kind === 'string' && atom.kind.length > 0) return atom.kind;
  return 'center';
}

/**
 * Build an anchor point per distinct group. Named zones map to their fixed
 * region; unknown groups spread deterministically around an ellipse (the
 * largest unknown group, or any named "center"/"room" group, takes the
 * middle). This is what makes the layout a constellation, not a blob.
 */
function buildGroupAnchors(
  atoms: readonly Atom[],
  groups: Record<string, string> | undefined,
): Map<string, { x: number; y: number }> {
  const counts = new Map<string, number>();
  for (const atom of atoms) {
    const group = resolveGroup(atom, groups);
    counts.set(group, (counts.get(group) ?? 0) + 1);
  }

  const anchors = new Map<string, { x: number; y: number }>();
  const unknown: string[] = [];
  for (const group of counts.keys()) {
    const named = NAMED_ZONES[group];
    if (named) {
      anchors.set(group, { x: named.x * VW, y: named.y * VH });
    } else {
      unknown.push(group);
    }
  }

  // The biggest unknown group claims center (unless a named center exists).
  unknown.sort((a, b) => (counts.get(b) ?? 0) - (counts.get(a) ?? 0) || a.localeCompare(b));
  const hasNamedCenter = anchors.has('center') || anchors.has('room');
  unknown.forEach((group, index) => {
    if (index === 0 && !hasNamedCenter) {
      anchors.set(group, { x: VW / 2, y: VH * 0.48 });
      return;
    }
    const ringIndex = hasNamedCenter ? index : index - 1;
    const ringCount = Math.max(1, hasNamedCenter ? unknown.length : unknown.length - 1);
    const angle = (ringIndex / ringCount) * Math.PI * 2 - Math.PI / 2;
    anchors.set(group, {
      x: VW / 2 + Math.cos(angle) * VW * 0.3,
      y: VH / 2 + Math.sin(angle) * VH * 0.32,
    });
  });

  return anchors;
}

function clamp01(value: number): number {
  if (value < 0) return 0;
  if (value > 1) return 1;
  return value;
}

function positiveOr(value: number | undefined, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : fallback;
}
