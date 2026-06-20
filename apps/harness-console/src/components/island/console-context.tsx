"use client";

import * as React from "react";

/**
 * Console-wide UI state that the Dynamic Island reads. This is the Civic Atlas
 * ambient logic seam: the bar reflects the most salient thing on screen. A
 * content surface feeds it `activeSection` (scroll-spy) and a TOC; a graph
 * surface feeds it `hoverNode` to override the ambient content, and registers
 * `surfaceMode: "memory"` so expanding shows the cluster list instead of a TOC.
 */

export interface TocEntry {
  id: string;
  title: string;
  depth: number;
}

export interface HoverNode {
  title: string;
  meta?: string; // kind, confidence, edge count, whatever the node carries
}

export type SurfaceMode = "content" | "memory";

interface ConsoleState {
  paletteOpen: boolean;
  setPaletteOpen: (v: boolean) => void;

  searchOn: boolean;
  setSearchOn: (v: boolean) => void;

  activeSection: string | null;
  setActiveSection: (id: string | null) => void;
  progress: number; // 0..1 scroll progress for the ring
  setProgress: (n: number) => void;

  toc: TocEntry[];
  setToc: (t: TocEntry[]) => void;

  surfaceMode: SurfaceMode;
  setSurfaceMode: (m: SurfaceMode) => void;

  clusters: { id: string; label: string }[];
  setClusters: (c: { id: string; label: string }[]) => void;

  hoverNode: HoverNode | null;
  setHoverNode: (n: HoverNode | null) => void;
}

const ConsoleContext = React.createContext<ConsoleState | null>(null);

export function ConsoleProvider({ children }: { children: React.ReactNode }) {
  const [paletteOpen, setPaletteOpen] = React.useState(false);
  const [searchOn, setSearchOn] = React.useState(false);
  const [activeSection, setActiveSection] = React.useState<string | null>(null);
  const [progress, setProgress] = React.useState(0);
  const [toc, setToc] = React.useState<TocEntry[]>([]);
  const [surfaceMode, setSurfaceMode] = React.useState<SurfaceMode>("content");
  const [clusters, setClusters] = React.useState<{ id: string; label: string }[]>([]);
  const [hoverNode, setHoverNode] = React.useState<HoverNode | null>(null);

  // Cmd/Ctrl K opens the command palette from anywhere; Escape collapses.
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      }
      if (e.key === "Escape") {
        setPaletteOpen(false);
        setSearchOn(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const value: ConsoleState = {
    paletteOpen,
    setPaletteOpen,
    searchOn,
    setSearchOn,
    activeSection,
    setActiveSection,
    progress,
    setProgress,
    toc,
    setToc,
    surfaceMode,
    setSurfaceMode,
    clusters,
    setClusters,
    hoverNode,
    setHoverNode,
  };

  return <ConsoleContext.Provider value={value}>{children}</ConsoleContext.Provider>;
}

export function useConsole(): ConsoleState {
  const ctx = React.useContext(ConsoleContext);
  if (!ctx) throw new Error("useConsole must be used within ConsoleProvider");
  return ctx;
}
