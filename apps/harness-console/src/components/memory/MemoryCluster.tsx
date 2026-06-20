"use client";

import * as React from "react";
import { Loader2, Focus, X } from "lucide-react";
import type { Atom, MemoryEdge, MemoryCluster as ClusterT } from "@/lib/harness";
import { CosmosGraph, type GraphNode, type GraphLink } from "@/components/graph/CosmosGraph";
import { useConsole } from "@/components/island/console-context";
import { EmptyState } from "@/components/common/EmptyState";
import { hueToRgb01 } from "./MemoryGraph";
import { cn } from "@/lib/utils";

/**
 * Memory Cluster view: the recent.design look applied to memory (Lane A). The
 * same CosmosGraph, but driven by the precomputed UMAP x/y as a cluster
 * projection, colored by cluster, with floating cluster labels to the side as
 * the navigation, mirroring the list the Dynamic Island shows when expanded on
 * this surface. Clicking a label frames/filters that cluster; clicking a node
 * opens the shared editor. Empty / computing / single-cluster-focus are all
 * rendered states.
 */
export function MemoryCluster({
  atoms,
  edges,
  clusters,
  loading,
  onNodeClick,
  className,
}: {
  atoms: Atom[];
  edges: MemoryEdge[];
  clusters: ClusterT[];
  loading?: boolean;
  onNodeClick: (atom: Atom) => void;
  className?: string;
}) {
  const { setHoverNode, setSurfaceMode, setClusters } = useConsole();
  const [focus, setFocus] = React.useState<string | null>(null);

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

  const atomById = React.useMemo(() => {
    const m = new Map<string, Atom>();
    atoms.forEach((a) => m.set(a.id, a));
    return m;
  }, [atoms]);

  const hueByCluster = React.useMemo(() => {
    const m = new Map<string, number>();
    clusters.forEach((c) => m.set(c.id, c.hue));
    return m;
  }, [clusters]);

  // Per-cluster live counts from the atoms actually present.
  const countByCluster = React.useMemo(() => {
    const m = new Map<string, number>();
    atoms.forEach((a) => m.set(a.clusterId ?? "", (m.get(a.clusterId ?? "") ?? 0) + 1));
    return m;
  }, [atoms]);

  // Focus filters the projection to one cluster; otherwise show all atoms but
  // dim the ones outside the hovered/selected cluster by alpha via size.
  const visibleAtoms = React.useMemo(
    () => (focus ? atoms.filter((a) => a.clusterId === focus) : atoms),
    [atoms, focus],
  );

  const nodes = React.useMemo<GraphNode[]>(
    () =>
      visibleAtoms.map((a) => ({
        id: a.id,
        x: a.x,
        y: a.y,
        color: hueToRgb01(hueByCluster.get(a.clusterId ?? "") ?? 0),
        size: 3 + a.salience * 5,
        label: a.title,
        meta: a.kind,
      })),
    [visibleAtoms, hueByCluster],
  );

  const links = React.useMemo<GraphLink[]>(() => {
    const present = new Set(visibleAtoms.map((a) => a.id));
    return edges
      .filter((e) => present.has(e.from) && present.has(e.to))
      .map((e) => ({ source: e.from, target: e.to }));
  }, [visibleAtoms, edges]);

  return (
    <div className={cn("relative", className)}>
      {/* The projection. */}
      {loading ? (
        <div className="grid h-full place-items-center rounded-lg border border-dashed border-line">
          <div className="flex flex-col items-center gap-2 text-muted-foreground">
            <Loader2 size={20} className="animate-[spin_1s_linear_infinite]" />
            <span className="font-mono text-label">computing the cluster projection...</span>
          </div>
        </div>
      ) : nodes.length === 0 ? (
        <EmptyState
          title="Nothing to cluster"
          description="No atoms match the current filters, so there is no projection to draw."
        />
      ) : (
        <CosmosGraph
          className="h-full w-full rounded-lg border border-line bg-surface"
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
              setHoverNode({ title: a.title, meta: `${a.kind} | ${cluster?.label ?? "unclustered"}` });
            }
          }}
          onNodeOut={() => setHoverNode(null)}
        />
      )}

      {/* Floating cluster labels to the side: the navigation. */}
      {!loading && nodes.length > 0 && (
        <div className="pointer-events-none absolute right-3 top-3 flex w-52 flex-col gap-1.5">
          <div className="material pointer-events-auto flex flex-col gap-1 p-2">
            <div className="rail-group-label mb-1 flex items-center justify-between px-1">
              <span>clusters</span>
              {focus && (
                <button
                  onClick={() => setFocus(null)}
                  className="inline-flex items-center gap-1 font-mono text-[10px] text-ox hover:underline"
                  title="Clear focus"
                >
                  <X size={10} /> clear
                </button>
              )}
            </div>
            {clusters.map((c) => {
              const rgb = hueToRgb01(c.hue);
              const swatch = `rgb(${Math.round(rgb[0] * 255)}, ${Math.round(rgb[1] * 255)}, ${Math.round(rgb[2] * 255)})`;
              const active = focus === c.id;
              const count = countByCluster.get(c.id) ?? 0;
              return (
                <button
                  key={c.id}
                  onClick={() => setFocus((f) => (f === c.id ? null : c.id))}
                  onMouseEnter={() => setHoverNode({ title: c.label, meta: `${count} atoms` })}
                  onMouseLeave={() => setHoverNode(null)}
                  className={cn(
                    "flex items-center gap-2 rounded px-2 py-1 text-left transition-colors",
                    active ? "bg-[var(--ox-tint)]" : "hover:bg-surface-2",
                  )}
                  aria-pressed={active}
                >
                  <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: swatch }} />
                  <span className={cn("flex-1 truncate text-label", active ? "text-ink" : "text-muted-foreground")}>
                    {c.label}
                  </span>
                  <span className="font-mono text-[10px] text-faint">{count}</span>
                  {active && <Focus size={11} className="text-ox" />}
                </button>
              );
            })}
          </div>
          {focus && (
            <div className="material pointer-events-auto p-2">
              <p className="px-1 font-mono text-[10px] leading-relaxed text-muted-foreground">
                framed to{" "}
                <span className="text-ink">{clusters.find((c) => c.id === focus)?.label}</span>. click a node to
                edit, or clear to see the whole field.
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
