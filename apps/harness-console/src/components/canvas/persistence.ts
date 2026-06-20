"use client";

import type { Node, Edge, Viewport } from "@xyflow/react";

/**
 * Canvas persistence in the JSON Canvas format (jsoncanvas.org, the same open
 * format Obsidian Canvas uses, so it interops with the obsidian-sync work). The
 * arrangement is stored locally so a user's canvas is exactly the way the screen
 * appeared the last time they opened the site. The viewport (pan + zoom) is
 * persisted alongside the spec nodes/edges so the restore is pixel-faithful.
 *
 * Durable backing (the harness graph, synced across devices) is the named
 * follow-up; the local store is the source of truth for this slice.
 */

export interface CanvasCardData {
  title: string;
  body?: string;
  kind?: string;
  atomId?: string; // links the card back to a memory atom
  color?: string;
  [key: string]: unknown;
}

export type CanvasNode = Node<CanvasCardData>;
export type CanvasEdge = Edge;

// --- JSON Canvas spec shapes -------------------------------------------------
interface JCNode {
  id: string;
  type: "text" | "file" | "link" | "group";
  x: number;
  y: number;
  width: number;
  height: number;
  color?: string;
  text?: string;
  // non-spec metadata we round-trip so cards keep their title/kind/atom link
  meta?: CanvasCardData;
}
interface JCEdge {
  id: string;
  fromNode: string;
  toNode: string;
  fromSide?: "top" | "right" | "bottom" | "left";
  toSide?: "top" | "right" | "bottom" | "left";
  label?: string;
}
interface JsonCanvas {
  nodes: JCNode[];
  edges: JCEdge[];
}
interface StoredCanvas {
  canvas: JsonCanvas;
  viewport?: Viewport;
}

const DEFAULT_W = 240;
const DEFAULT_H = 132;

function key(tenant: string) {
  return `harness-canvas:${tenant}`;
}

// --- React Flow <-> JSON Canvas ---------------------------------------------
export function toJsonCanvas(nodes: CanvasNode[], edges: CanvasEdge[]): JsonCanvas {
  return {
    nodes: nodes.map((n) => ({
      id: n.id,
      type: "text",
      x: Math.round(n.position.x),
      y: Math.round(n.position.y),
      width: Math.round((n.width as number) ?? (n.measured?.width as number) ?? DEFAULT_W),
      height: Math.round((n.height as number) ?? (n.measured?.height as number) ?? DEFAULT_H),
      color: n.data.color,
      text: n.data.body,
      meta: n.data,
    })),
    edges: edges.map((e) => ({ id: e.id, fromNode: e.source, toNode: e.target, label: e.label as string | undefined })),
  };
}

export function fromJsonCanvas(jc: JsonCanvas): { nodes: CanvasNode[]; edges: CanvasEdge[] } {
  return {
    nodes: jc.nodes.map((n) => ({
      id: n.id,
      type: "card",
      position: { x: n.x, y: n.y },
      style: { width: n.width, height: n.height },
      data: n.meta ?? { title: "Card", body: n.text, color: n.color },
    })),
    edges: jc.edges.map((e) => ({ id: e.id, source: e.fromNode, target: e.toNode, label: e.label })),
  };
}

// --- localStorage I/O --------------------------------------------------------
export function loadCanvas(tenant: string): { nodes: CanvasNode[]; edges: CanvasEdge[]; viewport?: Viewport } | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(key(tenant));
    if (!raw) return null;
    const stored = JSON.parse(raw) as StoredCanvas;
    const { nodes, edges } = fromJsonCanvas(stored.canvas);
    return { nodes, edges, viewport: stored.viewport };
  } catch {
    return null;
  }
}

export function saveCanvas(tenant: string, nodes: CanvasNode[], edges: CanvasEdge[], viewport?: Viewport): void {
  if (typeof window === "undefined") return;
  try {
    const stored: StoredCanvas = { canvas: toJsonCanvas(nodes, edges), viewport };
    window.localStorage.setItem(key(tenant), JSON.stringify(stored));
  } catch {
    /* quota or serialization failure: keep the session arrangement in memory */
  }
}

export function clearCanvas(tenant: string): void {
  if (typeof window === "undefined") return;
  window.localStorage.removeItem(key(tenant));
}

/** Export the current arrangement as a downloadable .canvas (JSON Canvas) file. */
export function exportJsonCanvas(nodes: CanvasNode[], edges: CanvasEdge[]): string {
  // strip our non-spec meta for a clean, portable .canvas file
  const jc = toJsonCanvas(nodes, edges);
  return JSON.stringify(
    { nodes: jc.nodes.map(({ meta, ...n }) => ({ ...n, text: meta?.title ? `${meta.title}\n\n${n.text ?? ""}` : n.text })), edges: jc.edges },
    null,
    2,
  );
}
