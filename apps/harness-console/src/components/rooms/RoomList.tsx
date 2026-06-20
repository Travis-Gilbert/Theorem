"use client";

import { cn, relativeTime } from "@/lib/utils";
import { StatusDot } from "@/components/ui/status-dot";
import { Skeleton } from "@/components/ui/misc";
import { EmptyState } from "@/components/common/EmptyState";
import { Inbox } from "lucide-react";
import type { Room, Presence } from "@/lib/harness";

/**
 * The room rail: a selectable list of coordination rooms. Each row shows the
 * room name, topic, a stack of presence dots for participants, and when it was
 * last touched. Selecting a row drives the detail pane.
 */

/** A room is "live" if any participant is present; that drives the rail dot. */
function roomPresence(room: Room): Presence | null {
  const order: Presence[] = ["live", "idle", "away"];
  for (const p of order) {
    if (room.participants.some((x) => x.presence === p)) return p;
  }
  return null;
}

export function RoomList({
  rooms,
  loading,
  selectedId,
  onSelect,
}: {
  rooms: Room[] | null;
  loading: boolean;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  if (loading) {
    return (
      <div className="flex flex-col gap-2" aria-busy="true">
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton key={i} className="h-[72px] w-full rounded-lg" />
        ))}
      </div>
    );
  }

  if (!rooms || rooms.length === 0) {
    return (
      <EmptyState
        icon={Inbox}
        title="No coordination rooms"
        description="Rooms appear here when heads announce intent, post records, or mention each other over the shared substrate."
      />
    );
  }

  return (
    <ul className="flex flex-col gap-2" role="listbox" aria-label="Coordination rooms">
      {rooms.map((room) => {
        const active = room.id === selectedId;
        const presence = roomPresence(room);
        return (
          <li key={room.id}>
            <button
              type="button"
              role="option"
              aria-selected={active}
              onClick={() => onSelect(room.id)}
              className={cn(
                "group w-full rounded-lg border px-3 py-3 text-left transition-colors",
                active
                  ? "border-[var(--ox)] bg-[var(--ox-tint)]"
                  : "border-line bg-surface hover:bg-surface-2",
              )}
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-2">
                  {presence && <StatusDot status={presence} pulse={presence === "live"} />}
                  <span className="truncate font-mono text-label text-ink">{room.name}</span>
                </div>
                <span className="shrink-0 font-mono text-[11px] text-faint">{relativeTime(room.updated)}</span>
              </div>
              <p className="mt-1 line-clamp-2 text-label text-muted-foreground">{room.topic}</p>
              <div className="mt-2 flex items-center gap-1.5">
                {room.participants.map((p) => (
                  <span key={p.actor} className="flex items-center gap-1" title={`${p.actor} - ${p.presence}`}>
                    <StatusDot status={p.presence} />
                    <span className="font-mono text-[11px] text-faint">{p.actor}</span>
                  </span>
                ))}
              </div>
            </button>
          </li>
        );
      })}
    </ul>
  );
}
