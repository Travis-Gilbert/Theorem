"use client";

import * as React from "react";
import { Graph } from "@cosmos.gl/graph";

/**
 * Lane A, the GPU lane: a thin React wrapper over @cosmos.gl/graph v3. Used by
 * Memory explore mode and the memory cluster, and reusable anywhere a large
 * graph is needed. cosmos.gl is a GPU force-simulation engine, not a generic
 * graph library: the data contract is typed arrays (positions, colors, links).
 *
 * When nodes carry precomputed coordinates (UMAP x/y from the cluster pipeline)
 * the layout is held static; otherwise the force simulation runs. WebGL is
 * required, so the component mounts client-only and degrades to a message if a
 * context cannot be created.
 */

export interface GraphNode {
  id: string;
  x?: number; // 0..1 normalized (e.g. UMAP projection)
  y?: number;
  color?: [number, number, number]; // 0..1 rgb
  size?: number;
  label?: string;
  meta?: string;
}

export interface GraphLink {
  source: string;
  target: string;
}

const SPACE = 4096;

export function CosmosGraph({
  nodes,
  links,
  onNodeClick,
  onNodeHover,
  onNodeOut,
  className,
}: {
  nodes: GraphNode[];
  links: GraphLink[];
  onNodeClick?: (node: GraphNode) => void;
  onNodeHover?: (node: GraphNode) => void;
  onNodeOut?: () => void;
  className?: string;
}) {
  const containerRef = React.useRef<HTMLDivElement>(null);
  const graphRef = React.useRef<Graph | null>(null);
  const [failed, setFailed] = React.useState(false);

  // Stable index map for click/hover routing.
  const indexById = React.useMemo(() => {
    const m = new Map<string, number>();
    nodes.forEach((n, i) => m.set(n.id, i));
    return m;
  }, [nodes]);

  React.useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const hasCoords = nodes.every((n) => typeof n.x === "number" && typeof n.y === "number");

    let graph: Graph;
    try {
      graph = new Graph(el, {
        spaceSize: SPACE,
        backgroundColor: [0, 0, 0, 0],
        pointSizeScale: 1,
        simulationGravity: hasCoords ? 0 : 0.25,
        simulationRepulsion: hasCoords ? 0 : 0.6,
        simulationLinkDistance: 12,
        enableDrag: true,
        fitViewOnInit: true,
        onPointClick: (index) => {
          const n = nodes[index];
          if (n) onNodeClick?.(n);
        },
        onPointMouseOver: (index) => {
          const n = nodes[index];
          if (n) onNodeHover?.(n);
        },
        onPointMouseOut: () => onNodeOut?.(),
      });
    } catch {
      setFailed(true);
      return;
    }
    graphRef.current = graph;

    const positions = new Float32Array(nodes.length * 2);
    const colors = new Float32Array(nodes.length * 4);
    const sizes = new Float32Array(nodes.length);
    nodes.forEach((n, i) => {
      positions[i * 2] = (n.x ?? Math.random()) * SPACE;
      positions[i * 2 + 1] = (n.y ?? Math.random()) * SPACE;
      const [r, g, b] = n.color ?? [0.43, 0.18, 0.16];
      colors[i * 4] = r;
      colors[i * 4 + 1] = g;
      colors[i * 4 + 2] = b;
      colors[i * 4 + 3] = 0.9;
      sizes[i] = n.size ?? 4;
    });
    graph.setPointPositions(positions);
    graph.setPointColors(colors);
    graph.setPointSizes(sizes);

    if (links.length) {
      const linkArr = new Float32Array(links.length * 2);
      links.forEach((l, i) => {
        linkArr[i * 2] = indexById.get(l.source) ?? 0;
        linkArr[i * 2 + 1] = indexById.get(l.target) ?? 0;
      });
      graph.setLinks(linkArr);
    }

    graph.render(hasCoords ? 0 : 0.6);

    return () => {
      graph.destroy();
      graphRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nodes, links]);

  if (failed) {
    return (
      <div className={className}>
        <div className="grid h-full place-items-center rounded-lg border border-dashed border-line text-label text-muted-foreground">
          WebGL is unavailable in this context; the graph needs a GPU surface.
        </div>
      </div>
    );
  }

  return <div ref={containerRef} className={className} style={{ width: "100%", height: "100%" }} />;
}
