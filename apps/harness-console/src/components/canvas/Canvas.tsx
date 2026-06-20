"use client";

import "@xyflow/react/dist/style.css";
import * as React from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  BackgroundVariant,
  Controls,
  Panel,
  Handle,
  Position,
  addEdge,
  useNodesState,
  useEdgesState,
  useReactFlow,
  type Connection,
  type NodeProps,
  type Viewport,
} from "@xyflow/react";
import { Plus, RotateCcw, Download, ExternalLink } from "lucide-react";
import Link from "next/link";
import { harness, HARNESS_SOURCE } from "@/lib/harness";
import { Badge } from "@/components/ui/badge";
import {
  type CanvasNode,
  type CanvasEdge,
  loadCanvas,
  saveCanvas,
  clearCanvas,
  exportJsonCanvas,
} from "./persistence";

const TENANT = process.env.NEXT_PUBLIC_DEFAULT_TENANT ?? "default";
const DEFAULT_VIEWPORT: Viewport = { x: 0, y: 0, zoom: 1 };

/* ----------------------------------------------------------------------------
 * Card node: an atom card (read-only, links to Memory) or an editable note.
 * ------------------------------------------------------------------------- */
function CardNode({ id, data }: NodeProps<CanvasNode>) {
  const { setNodes } = useReactFlow();
  const editable = !data.atomId;

  const setBody = (body: string) =>
    setNodes((ns) => ns.map((n) => (n.id === id ? { ...n, data: { ...n.data, body } } : n)));

  return (
    <div className="material flex h-full w-full flex-col gap-1.5 p-3">
      <Handle type="target" position={Position.Left} className="!h-2 !w-2 !border-0 !bg-ox" />
      <div className="flex items-center justify-between gap-2">
        <span className="truncate font-title text-[13px] text-ink">{data.title}</span>
        {data.kind && <Badge tone="neutral">{data.kind}</Badge>}
      </div>
      {editable ? (
        <textarea
          value={data.body ?? ""}
          onChange={(e) => setBody(e.target.value)}
          placeholder="Write a note..."
          className="nodrag nowheel min-h-0 flex-1 resize-none bg-transparent font-mono text-[11px] leading-relaxed text-muted-foreground outline-none placeholder:text-faint"
        />
      ) : (
        <p className="min-h-0 flex-1 overflow-hidden font-mono text-[11px] leading-relaxed text-muted-foreground">
          {data.body}
        </p>
      )}
      {data.atomId && (
        <Link
          href={`/memory?atom=${data.atomId}`}
          className="nodrag inline-flex items-center gap-1 font-mono text-[10px] text-ox hover:underline"
        >
          <ExternalLink size={10} /> open in memory
        </Link>
      )}
      <Handle type="source" position={Position.Right} className="!h-2 !w-2 !border-0 !bg-ox" />
    </div>
  );
}

const nodeTypes = { card: CardNode };

/* ----------------------------------------------------------------------------
 * Canvas
 * ------------------------------------------------------------------------- */
function CanvasInner() {
  // Canvas renders client-only (dynamic ssr:false), so the saved arrangement is
  // read synchronously at init: the stored branch needs no setState in an effect,
  // and the initial viewport is a plain value rather than a ref read during render.
  const initial = React.useMemo(() => loadCanvas(TENANT), []);
  const [nodes, setNodes, onNodesChange] = useNodesState<CanvasNode>(initial?.nodes ?? []);
  const [edges, setEdges, onEdgesChange] = useEdgesState<CanvasEdge>(initial?.edges ?? []);
  const [ready, setReady] = React.useState(!!initial);
  const viewportRef = React.useRef<Viewport>(initial?.viewport ?? DEFAULT_VIEWPORT);
  const saveTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const counter = React.useRef(0);

  // No saved arrangement yet: seed a starter from recent memory atoms. Every
  // state update happens in the async resolution, not synchronously in the effect.
  React.useEffect(() => {
    if (initial) return;
    let cancelled = false;
    harness
      .listMemory({ view: "active" })
      .then((list) => {
        if (cancelled) return;
        const seed: CanvasNode[] = list.atoms.slice(0, 6).map((a, i) => ({
          id: a.id,
          type: "card",
          position: { x: 60 + (i % 3) * 280, y: 60 + Math.floor(i / 3) * 180 },
          style: { width: 240, height: 132 },
          data: { title: a.title, body: a.summary, kind: a.kind, atomId: a.id },
        }));
        const welcome: CanvasNode = {
          id: "welcome",
          type: "card",
          position: { x: 60, y: 460 },
          style: { width: 360, height: 120 },
          data: {
            title: "Your canvas",
            body: "Drag cards to arrange them, connect them, add notes. It is saved exactly as you leave it and restored next time you open the site.",
          },
        };
        setNodes([welcome, ...seed]);
        saveCanvas(TENANT, [welcome, ...seed], [], viewportRef.current);
        setReady(true);
      })
      .catch(() => setReady(true));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Persist (debounced) on any structural change.
  React.useEffect(() => {
    if (!ready) return;
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => saveCanvas(TENANT, nodes, edges, viewportRef.current), 400);
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    };
  }, [nodes, edges, ready]);

  const onConnect = React.useCallback(
    (c: Connection) => setEdges((eds) => addEdge({ ...c, id: `e_${counter.current++}_${Date.now() % 100000}` }, eds)),
    [setEdges],
  );

  const addCard = () => {
    const vp = viewportRef.current;
    const id = `note_${counter.current++}_${Date.now() % 100000}`;
    setNodes((ns) => [
      ...ns,
      {
        id,
        type: "card",
        position: { x: -vp.x / vp.zoom + 220, y: -vp.y / vp.zoom + 160 },
        style: { width: 240, height: 132 },
        data: { title: "Note", body: "" },
      },
    ]);
  };

  const resetCanvas = () => {
    clearCanvas(TENANT);
    setNodes([]);
    setEdges([]);
  };

  const exportCanvas = () => {
    const blob = new Blob([exportJsonCanvas(nodes, edges)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "harness.canvas";
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      onConnect={onConnect}
      onMove={(_, vp) => (viewportRef.current = vp)}
      onMoveEnd={(_, vp) => {
        viewportRef.current = vp;
        if (ready) saveCanvas(TENANT, nodes, edges, vp);
      }}
      nodeTypes={nodeTypes}
      defaultViewport={initial?.viewport ?? DEFAULT_VIEWPORT}
      minZoom={0.2}
      maxZoom={2.5}
      deleteKeyCode={["Backspace", "Delete"]}
      proOptions={{ hideAttribution: false }}
      className="bg-transparent"
    >
      <Background variant={BackgroundVariant.Dots} gap={28} size={1} color="var(--line)" />
      <Controls className="!border !border-line !bg-bg !shadow-[var(--elev-1)]" showInteractive={false} />
      <Panel position="top-left" className="flex items-center gap-1.5">
        <button
          onClick={addCard}
          className="inline-flex items-center gap-1.5 rounded-md bg-ox px-2.5 py-1.5 font-mono text-[11px] text-white hover:bg-[#73241f]"
        >
          <Plus size={13} /> Add card
        </button>
        <button
          onClick={exportCanvas}
          className="inline-flex items-center gap-1.5 rounded-md border border-line bg-bg px-2.5 py-1.5 font-mono text-[11px] text-ink hover:bg-surface-2"
        >
          <Download size={13} /> Export .canvas
        </button>
        <button
          onClick={resetCanvas}
          className="inline-flex items-center gap-1.5 rounded-md border border-line bg-bg px-2.5 py-1.5 font-mono text-[11px] text-muted-foreground hover:bg-surface-2 hover:text-ink"
        >
          <RotateCcw size={13} /> Reset
        </button>
      </Panel>
      <Panel position="top-right">
        <div className="rounded-md border border-line bg-bg/80 px-2 py-1 font-mono text-[10px] text-muted-foreground backdrop-blur">
          arrangement saved locally {HARNESS_SOURCE === "live" ? "(durable backing: follow-up)" : ""}
        </div>
      </Panel>
    </ReactFlow>
  );
}

export function Canvas() {
  return (
    <div className="h-full w-full">
      <ReactFlowProvider>
        <CanvasInner />
      </ReactFlowProvider>
    </div>
  );
}
