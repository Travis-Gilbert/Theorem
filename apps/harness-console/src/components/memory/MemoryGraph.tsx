"use client";

import * as React from "react";
import type { Atom, MemoryEdge, MemoryCluster } from "@/lib/harness";
import { CosmosGraph, type GraphNode, type GraphLink } from "@/components/graph/CosmosGraph";
import { useConsole } from "@/components/island/console-context";

/**
 * Convert an HSL hue (0..360) to an rgb 0..1 triple for cosmos.gl. Fixed
 * saturation/lightness keep the per-cluster hues legible against the white field
 * without any one cluster reading as the oxblood accent.
 */
export function hueToRgb01(hue: number, sat = 0.55, light = 0.5): [number, number, number] {
  const h = ((hue % 360) + 360) % 360;
  const c = (1 - Math.abs(2 * light - 1)) * sat;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = light - c / 2;
  let r = 0;
  let g = 0;
  let b = 0;
  if (h < 60) [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  return [r + m, g + m, b + m];
}

/**
 * Graph (explore) mode: the SAME atoms and the SAME editor, projected as a GPU
 * force/UMAP graph. Nodes are atoms colored by cluster hue; links are the memory
 * edges. Clicking a node opens the editor (onNodeClick). Hover feeds the Dynamic
 * Island ambient pill (setHoverNode); the surface registers as "memory" so
 * expanding the island shows the cluster list instead of a TOC.
 */
export function MemoryGraph({
  atoms,
  edges,
  clusters,
  onNodeClick,
  className,
}: {
  atoms: Atom[];
  edges: MemoryEdge[];
  clusters: MemoryCluster[];
  onNodeClick: (atom: Atom) => void;
  className?: string;
}) {
  const { setHoverNode, setSurfaceMode, setClusters } = useConsole();

  const hueByCluster = React.useMemo(() => {
    const m = new Map<string, number>();
    clusters.forEach((c) => m.set(c.id, c.hue));
    return m;
  }, [clusters]);

  const atomById = React.useMemo(() => {
    const m = new Map<string, Atom>();
    atoms.forEach((a) => m.set(a.id, a));
    return m;
  }, [atoms]);

  const nodes = React.useMemo<GraphNode[]>(
    () =>
      atoms.map((a) => ({
        id: a.id,
        x: a.x,
        y: a.y,
        color: hueToRgb01(hueByCluster.get(a.clusterId ?? "") ?? 0),
        size: 3 + a.salience * 4,
        label: a.title,
        meta: a.kind,
      })),
    [atoms, hueByCluster],
  );

  // Only links whose endpoints are both present in the current node set.
  const links = React.useMemo<GraphLink[]>(() => {
    const present = new Set(atoms.map((a) => a.id));
    return edges
      .filter((e) => present.has(e.from) && present.has(e.to))
      .map((e) => ({ source: e.from, target: e.to }));
  }, [atoms, edges]);

  // Register this as the memory surface so the island shows clusters, and feed
  // it the cluster list. Reset to the content surface on unmount.
  React.useEffect(() => {
    setSurfaceMode("memory");
    setClusters(clusters.map((c) => ({ id: c.id, label: c.label })));
    return () => {
      setSurfaceMode("content");
      setClusters([]);
      setHoverNode(null);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clusters]);

  return (
    <CosmosGraph
      className={className}
      nodes={nodes}
      links={links}
      onNodeClick={(n) => {
        const a = atomById.get(n.id);
        if (a) onNodeClick(a);
      }}
      onNodeHover={(n) => {
        const a = atomById.get(n.id);
        if (a) {
          const cluster = clusters.find((c) => c.id === a.clusterId);
          setHoverNode({
            title: a.title,
            meta: `${a.kind} | ${cluster?.label ?? "unclustered"} | salience ${a.salience.toFixed(2)}`,
          });
        }
      }}
      onNodeOut={() => setHoverNode(null)}
    />
  );
}
