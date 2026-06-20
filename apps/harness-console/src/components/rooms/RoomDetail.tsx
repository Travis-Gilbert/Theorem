"use client";

import * as React from "react";
import { cn, relativeTime } from "@/lib/utils";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { StatusDot } from "@/components/ui/status-dot";
import { Skeleton } from "@/components/ui/misc";
import { EmptyState } from "@/components/common/EmptyState";
import { useHarnessStream } from "@/lib/hooks/useHarnessStream";
import { MessagesSquare, AtSign, Radio } from "lucide-react";
import type { Room, RoomEvent, RoomEventKind } from "@/lib/harness";

/**
 * The room detail pane: live presence for participants, then a streaming event
 * feed. The feed seeds from the loaded room.events and then folds in synthetic
 * stream ticks from useHarnessStream so new coordination events arrive without a
 * reload. Mentions are surfaced inline and the @-targeted actors are summarized.
 */

const KIND_TONE: Record<RoomEventKind, "neutral" | "accent" | "live" | "warn" | "ink"> = {
  intent: "accent",
  message: "neutral",
  record: "live",
  decision: "ink",
  tension: "warn",
  reflection: "neutral",
  mention: "accent",
};

const STREAM_KINDS: RoomEventKind[] = ["intent", "message", "record", "reflection", "mention"];

/** Build a synthetic RoomEvent for the mock stream, scoped to this room's actors. */
function makeMockEvent(room: Room, tick: number): RoomEvent {
  const actors = room.participants.map((p) => p.actor);
  const actor = actors[tick % Math.max(1, actors.length)] ?? "claude-code";
  const kind = STREAM_KINDS[tick % STREAM_KINDS.length];
  const others = actors.filter((a) => a !== actor);
  const mention = kind === "mention" && others.length ? [others[tick % others.length]] : undefined;
  const lines: Record<RoomEventKind, string> = {
    intent: "Claiming the detail-pane seam; building the streaming feed.",
    message: "Palette and presence dots reconciled against the design tokens.",
    record: "Feed renders new events without a reload; acceptance met.",
    decision: "Read-only window: no writes from the console into rooms.",
    tension: "Tenant resolution still degraded on the coordinate endpoint.",
    reflection: "Holding the console lane; core crates stay with the other head.",
    mention: "Heads up on the shared event-feed contract.",
  };
  return {
    id: `stream_${room.id}_${tick}`,
    kind,
    actor,
    text: lines[kind],
    at: new Date().toISOString(),
    mentions: mention,
  };
}

function EventRow({ event }: { event: RoomEvent }) {
  return (
    <li className="flex gap-3 border-b border-line/60 px-4 py-3 last:border-b-0">
      <div className="pt-0.5">
        <Badge tone={KIND_TONE[event.kind]}>{event.kind}</Badge>
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-2">
          <span className="font-mono text-label text-ink">{event.actor}</span>
          <span className="shrink-0 font-mono text-[11px] text-faint">{relativeTime(event.at)}</span>
        </div>
        <p className="mt-0.5 text-body text-muted-foreground">{event.text}</p>
        {event.mentions && event.mentions.length > 0 && (
          <div className="mt-1.5 flex flex-wrap items-center gap-1">
            {event.mentions.map((m) => (
              <span key={m} className="inline-flex items-center gap-0.5 font-mono text-[11px] text-ox">
                <AtSign size={11} />
                {m}
              </span>
            ))}
          </div>
        )}
      </div>
    </li>
  );
}

function DetailSkeleton() {
  return (
    <div className="flex flex-col gap-4" aria-busy="true">
      <Skeleton className="h-24 w-full rounded-lg" />
      <Skeleton className="h-[420px] w-full rounded-lg" />
    </div>
  );
}

export function RoomDetail({ room, loading }: { room: Room | null; loading: boolean }) {
  // Stream synthetic events scoped to the selected room. Keyed by room id so it
  // resets when the selection changes.
  const { events: streamed, connected } = useHarnessStream<RoomEvent>(`room:${room?.id ?? "none"}`, {
    mockEvery: 3500,
    mockFactory: (tick) => makeMockEvent(room ?? ({ id: "none", participants: [] } as unknown as Room), tick),
  });

  // Seed from the room's loaded events, append streamed ticks, newest last.
  const feed = React.useMemo<RoomEvent[]>(() => {
    if (!room) return [];
    const base = [...room.events].sort((a, b) => new Date(a.at).getTime() - new Date(b.at).getTime());
    return [...base, ...streamed];
  }, [room, streamed]);

  // Auto-scroll the feed to the newest event as it streams in.
  const viewportRef = React.useRef<HTMLDivElement | null>(null);
  React.useEffect(() => {
    const el = viewportRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [feed.length]);

  if (loading) return <DetailSkeleton />;

  if (!room) {
    return (
      <EmptyState
        icon={MessagesSquare}
        title="Select a room"
        description="Pick a room from the rail to see live presence and the recent coordination feed."
      />
    );
  }

  const mentionCount = feed.reduce((n, e) => n + (e.mentions?.length ?? 0), 0);

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <CardTitle>{room.name}</CardTitle>
              <p className="mt-1 text-label text-muted-foreground">{room.topic}</p>
            </div>
            <Badge tone={connected ? "live" : "neutral"}>
              <Radio size={11} />
              {connected ? "live" : "offline"}
            </Badge>
          </div>
        </CardHeader>
        <CardContent>
          <div className="rail-group-label mb-2">presence</div>
          <ul className="flex flex-wrap gap-x-5 gap-y-2">
            {room.participants.map((p) => (
              <li key={p.actor} className="flex items-center gap-2">
                <StatusDot status={p.presence} pulse={p.presence === "live"} />
                <span className="font-mono text-label text-ink">{p.actor}</span>
                <span className="font-mono text-[11px] text-faint">{p.presence}</span>
                <span className="font-mono text-[11px] text-faint">- {relativeTime(p.lastSeen)}</span>
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>

      <Card calm>
        <div className="flex items-center justify-between border-b border-line px-4 py-3">
          <div className="flex items-center gap-2 font-mono text-label text-ink">
            <MessagesSquare size={14} className="text-muted-foreground" />
            event feed
          </div>
          <div className="flex items-center gap-2 font-mono text-[11px] text-faint">
            <span>{feed.length} events</span>
            {mentionCount > 0 && (
              <span className="inline-flex items-center gap-0.5 text-ox">
                <AtSign size={11} />
                {mentionCount}
              </span>
            )}
          </div>
        </div>
        {feed.length === 0 ? (
          <div className="px-4 py-10 text-center text-label text-muted-foreground">No events yet. Listening for activity.</div>
        ) : (
          <div ref={viewportRef} className="max-h-[440px] overflow-y-auto">
            <ul>
              {feed.map((event) => (
                <EventRow key={event.id} event={event} />
              ))}
            </ul>
          </div>
        )}
      </Card>
    </div>
  );
}
