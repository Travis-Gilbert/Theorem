import { useEffect, useMemo, useState, type ReactNode } from "react";
import type {
  RoomFeedItem,
  RoomIntent,
  RoomParticipant,
  RoomRecord,
} from "../state/types";

type ViewNodeKind = "room" | "agent" | "message" | "intent" | "record" | "footprint";
type ViewEdgeKind = "member" | "message" | "intent" | "footprint" | "record" | "conflict";

interface ViewNode {
  id: string;
  kind: ViewNodeKind;
  label: string;
  sublabel?: string;
  status?: string;
  summary?: string;
  x: number;
  y: number;
}

interface ViewEdge {
  id: string;
  source: string;
  target: string;
  kind: ViewEdgeKind;
  status?: string;
}

interface ViewGraph {
  nodes: ViewNode[];
  edges: ViewEdge[];
}

interface AgentSpaceViewportProps {
  roomId: string;
  participants: RoomParticipant[];
  feed: RoomFeedItem[];
  intents: RoomIntent[];
  records: RoomRecord[];
}

export function AgentSpaceViewport({
  roomId,
  participants,
  feed,
  intents,
  records,
}: AgentSpaceViewportProps) {
  const rawGraph = useMemo(
    () => buildGraph(roomId, participants, feed, intents, records),
    [roomId, participants, feed, intents, records],
  );
  const graph = useFrameGraph(rawGraph);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => {
    if (selectedId && !graph.nodes.some((node) => node.id === selectedId)) {
      setSelectedId(null);
    }
  }, [graph.nodes, selectedId]);

  const nodeById = useMemo(
    () => new Map(graph.nodes.map((node) => [node.id, node])),
    [graph.nodes],
  );
  const selected = selectedId ? nodeById.get(selectedId) : undefined;
  const tensions = records
    .filter((record) => matchesAny(record.kind, record.summary, record.body, ["tension", "conflict"]))
    .slice(0, 4);
  const deltas = records
    .filter((record) => matchesAny(record.kind, record.summary, record.body, ["delta", "crdt"]))
    .slice(0, 4);
  const hasActivity = participants.length + feed.length + intents.length + records.length > 0;

  return (
    <section className="agent-space" aria-label="Agent space viewport">
      <div className="agent-space__viewport">
        <div className="agent-space__meta">
          <span>{roomId}</span>
          <span>
            {graph.nodes.length} nodes / {graph.edges.length} links
          </span>
        </div>
        <svg
          className="agent-space__svg"
          viewBox="0 0 420 270"
          role="img"
          aria-label={`Live coordination graph for ${roomId}`}
        >
          <defs>
            <marker
              id="agent-space-arrow"
              viewBox="0 0 10 10"
              refX="8"
              refY="5"
              markerWidth="5"
              markerHeight="5"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 10 5 L 0 10 z" />
            </marker>
          </defs>
          {graph.edges.map((edge) => {
            const source = nodeById.get(edge.source);
            const target = nodeById.get(edge.target);
            if (!source || !target) return null;
            return (
              <line
                key={edge.id}
                className={`agent-space__edge agent-space__edge--${edge.kind}`}
                x1={source.x}
                y1={source.y}
                x2={target.x}
                y2={target.y}
              />
            );
          })}
          {graph.nodes.map((node) => (
            <g
              key={node.id}
              className={[
                "agent-space__node",
                `agent-space__node--${node.kind}`,
                node.status ? `agent-space__node--${classToken(node.status)}` : "",
                selectedId === node.id ? "agent-space__node--selected" : "",
              ]
                .filter(Boolean)
                .join(" ")}
              transform={`translate(${node.x} ${node.y})`}
              tabIndex={0}
              role="button"
              aria-label={`${node.kind}: ${node.label}`}
              onClick={() => setSelectedId(node.id)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  setSelectedId(node.id);
                }
              }}
            >
              <title>{node.summary || node.label}</title>
              <circle r={node.kind === "room" ? 19 : node.kind === "agent" ? 14 : 11} />
              <text y={node.kind === "room" ? 34 : 27}>{node.label}</text>
            </g>
          ))}
        </svg>
        {!hasActivity && <div className="agent-space__empty">No room activity yet.</div>}
      </div>

      <div className="agent-space__panels">
        <Panel title="Selection">
          {selected ? (
            <div className="agent-space__selected">
              <span className="agent-space__kind">{selected.kind}</span>
              <strong>{selected.label}</strong>
              {selected.sublabel && <span>{selected.sublabel}</span>}
              {selected.summary && <p>{selected.summary}</p>}
            </div>
          ) : (
            <span className="agent-space__muted">Pick a node.</span>
          )}
        </Panel>

        <Panel title="Work">
          {intents.length > 0 ? (
            intents.slice(0, 5).map((intent) => (
              <div className="agent-space__row" key={`${intent.actor}-${intent.updatedAt ?? intent.summary}`}>
                <span className="agent-space__actor">{intent.actor}</span>
                <span>{intent.status}</span>
                <span>{intent.summary || intent.task || "No summary"}</span>
              </div>
            ))
          ) : (
            <span className="agent-space__muted">No active intents.</span>
          )}
        </Panel>

        <Panel title="Tensions">
          {tensions.length > 0 ? (
            tensions.map((record) => (
              <div className="agent-space__row" key={record.id}>
                <span className="agent-space__actor">{record.actor ?? "room"}</span>
                <span>{record.summary || record.title || record.kind}</span>
              </div>
            ))
          ) : (
            <span className="agent-space__muted">No tensions.</span>
          )}
        </Panel>

        <Panel title="Deltas">
          {deltas.length > 0 ? (
            deltas.map((record) => (
              <div className="agent-space__row" key={record.id}>
                <span className="agent-space__actor">{record.actor ?? "room"}</span>
                <span>{record.summary || record.title || record.kind}</span>
              </div>
            ))
          ) : (
            <span className="agent-space__muted">No CRDT deltas.</span>
          )}
        </Panel>
      </div>
    </section>
  );
}

function Panel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="agent-space__panel">
      <div className="agent-space__panel-title">{title}</div>
      {children}
    </section>
  );
}

function useFrameGraph(graph: ViewGraph): ViewGraph {
  const [framed, setFramed] = useState<ViewGraph>(graph);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => setFramed(graph));
    return () => window.cancelAnimationFrame(frame);
  }, [graph]);

  return framed;
}

function buildGraph(
  roomId: string,
  participants: RoomParticipant[],
  feed: RoomFeedItem[],
  intents: RoomIntent[],
  records: RoomRecord[],
): ViewGraph {
  const nodes = new Map<string, ViewNode>();
  const edges: ViewEdge[] = [];
  const roomNodeId = stableId("room", roomId || "unbound");

  const addNode = (node: ViewNode) => {
    nodes.set(node.id, node);
  };
  const addEdge = (edge: ViewEdge) => {
    if (edge.source !== edge.target) edges.push(edge);
  };

  addNode({
    id: roomNodeId,
    kind: "room",
    label: truncate(roomId || "room", 22),
    summary: "Bound coordination room",
    x: 205,
    y: 132,
  });

  const actors = uniqueStrings([
    ...participants.map((participant) => participant.actor),
    ...feed.map((item) => item.actor),
    ...intents.map((intent) => intent.actor),
    ...records.map((record) => record.actor ?? ""),
  ]).slice(0, 8);

  actors.forEach((actor, index) => {
    const participant = participants.find((item) => item.actor === actor);
    const nodeId = stableId("agent", actor);
    addNode({
      id: nodeId,
      kind: "agent",
      label: truncate(actor, 15),
      sublabel: participant?.status,
      status: participant?.status,
      summary: participant?.lastSeen ? `Last seen ${participant.lastSeen}` : participant?.status,
      x: 68,
      y: stackY(index, actors.length, 50, 218),
    });
    addEdge({
      id: stableId("member", actor, roomNodeId),
      source: nodeId,
      target: roomNodeId,
      kind: "member",
      status: participant?.status,
    });
  });

  intents.slice(0, 7).forEach((intent, index) => {
    const nodeId = stableId("intent", intent.actor, intent.updatedAt ?? index.toString());
    const actorId = stableId("agent", intent.actor);
    if (!nodes.has(actorId)) {
      addNode({
        id: actorId,
        kind: "agent",
        label: truncate(intent.actor, 15),
        status: intent.status,
        x: 68,
        y: stackY(actors.length + index, actors.length + intents.length, 50, 218),
      });
    }
    addNode({
      id: nodeId,
      kind: "intent",
      label: truncate(intent.status || "intent", 12),
      sublabel: truncate(intent.actor, 16),
      status: intent.status,
      summary: intent.summary || intent.task,
      x: 286,
      y: stackY(index, Math.min(intents.length, 7), 42, 116),
    });
    addEdge({
      id: stableId("intent-edge", actorId, nodeId),
      source: actorId,
      target: nodeId,
      kind: "intent",
      status: intent.status,
    });
    addEdge({
      id: stableId("intent-room", nodeId, roomNodeId),
      source: nodeId,
      target: roomNodeId,
      kind: "intent",
      status: intent.status,
    });
    intent.footprint.slice(0, 2).forEach((file, footprintIndex) => {
      const footprintId = stableId("footprint", file);
      addNode({
        id: footprintId,
        kind: "footprint",
        label: truncate(fileLabel(file), 16),
        summary: file,
        x: 370,
        y: stackY(index * 2 + footprintIndex, Math.min(intents.length * 2, 14), 38, 132),
      });
      addEdge({
        id: stableId("footprint-edge", nodeId, footprintId),
        source: nodeId,
        target: footprintId,
        kind: "footprint",
        status: intent.status,
      });
    });
  });

  feed.slice(-8).forEach((item, index) => {
    const nodeId = stableId("message", item.id || index.toString());
    const actorId = stableId("agent", item.actor);
    addNode({
      id: nodeId,
      kind: "message",
      label: truncate(item.actor, 12),
      sublabel: item.createdAt,
      summary: item.text,
      x: 300,
      y: stackY(index, Math.min(feed.length, 8), 146, 236),
    });
    addEdge({
      id: stableId("message-edge", actorId, nodeId),
      source: nodes.has(actorId) ? actorId : roomNodeId,
      target: nodeId,
      kind: "message",
    });
    addEdge({
      id: stableId("message-room", nodeId, roomNodeId),
      source: nodeId,
      target: roomNodeId,
      kind: "message",
    });
  });

  records.slice(0, 8).forEach((record, index) => {
    const nodeId = stableId("record", record.id || index.toString());
    const actorId = record.actor ? stableId("agent", record.actor) : roomNodeId;
    const conflict = matchesAny(record.kind, record.summary, record.body, ["tension", "conflict"]);
    addNode({
      id: nodeId,
      kind: "record",
      label: truncate(record.kind || "record", 13),
      sublabel: record.actor,
      status: conflict ? "contested" : record.kind,
      summary: record.summary || record.title || record.body,
      x: stackX(index, Math.min(records.length, 8), 126, 250),
      y: 232,
    });
    addEdge({
      id: stableId("record-edge", actorId, nodeId),
      source: nodes.has(actorId) ? actorId : roomNodeId,
      target: nodeId,
      kind: conflict ? "conflict" : "record",
      status: conflict ? "contested" : record.kind,
    });
  });

  return { nodes: Array.from(nodes.values()), edges };
}

function stackY(index: number, total: number, top: number, bottom: number): number {
  if (total <= 1) return (top + bottom) / 2;
  return top + ((bottom - top) * index) / (total - 1);
}

function stackX(index: number, total: number, left: number, right: number): number {
  if (total <= 1) return (left + right) / 2;
  return left + ((right - left) * index) / (total - 1);
}

function stableId(...parts: string[]): string {
  return parts
    .join(":")
    .replace(/[^a-zA-Z0-9_-]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 120);
}

function uniqueStrings(values: string[]): string[] {
  return Array.from(new Set(values.map((value) => value.trim()).filter(Boolean)));
}

function truncate(value: string, max: number): string {
  if (value.length <= max) return value;
  return `${value.slice(0, Math.max(1, max - 1))}.`;
}

function fileLabel(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? path;
}

function classToken(value: string): string {
  return stableId(value.toLowerCase()) || "unknown";
}

function matchesAny(
  kind: string | undefined,
  summary: string | undefined,
  body: string | undefined,
  terms: string[],
): boolean {
  const haystack = `${kind ?? ""} ${summary ?? ""} ${body ?? ""}`.toLowerCase();
  return terms.some((term) => haystack.includes(term));
}
