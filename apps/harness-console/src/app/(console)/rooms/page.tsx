"use client";

import * as React from "react";
import { harness } from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";
import { usePageToc } from "@/components/island/useScrollSpy";
import { PageHeader, Section } from "@/components/common/PageHeader";
import { RoomList } from "@/components/rooms/RoomList";
import { RoomDetail } from "@/components/rooms/RoomDetail";

/**
 * Rooms: a read-only window into coordination rooms. Master-detail: the room
 * rail on the left, the live detail (presence + streaming event feed) on the
 * right (stacked on narrow). Selecting a room loads its detail; the feed streams
 * new events in without a reload.
 */
export default function RoomsPage() {
  usePageToc();

  const { data: rooms, loading: roomsLoading } = useAsync(() => harness.listRooms(), []);
  const [picked, setPicked] = React.useState<string | null>(null);

  // The effective selection: an explicit pick, else default to the first room
  // once the list loads. Derived (no effect-driven setState) so selection never
  // lags the data, and a stale pick falls back gracefully.
  const selectedId =
    (picked && rooms?.some((r) => r.id === picked) ? picked : null) ?? rooms?.[0]?.id ?? null;

  const { data: room, loading: roomLoading } = useAsync(
    () => (selectedId ? harness.getRoom(selectedId) : Promise.resolve(null)),
    [selectedId],
  );

  return (
    <div>
      <PageHeader
        eyebrow="coordination"
        title="Rooms"
        description="A read-only window into the shared substrate: who is present, and the recent stream of intent, records, and mentions."
      />

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[320px_minmax(0,1fr)]">
        <Section id="rooms-list" title="Rooms" className="mb-0">
          <RoomList rooms={rooms} loading={roomsLoading} selectedId={selectedId} onSelect={setPicked} />
        </Section>

        <Section id="rooms-detail" title="Activity" className="mb-0">
          <RoomDetail room={room} loading={Boolean(selectedId) && roomLoading} />
        </Section>
      </div>
    </div>
  );
}
