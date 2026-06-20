"use client";

import { useEffect, useRef, useState } from "react";
import { HARNESS_SOURCE, HARNESS_URL } from "@/lib/harness";

/**
 * Shared SSE subscription manager for the Agent stream, Rooms presence/feed, and
 * the Runs ledger. In `mock` mode it emits synthetic ticks so the live surfaces
 * animate without a backend; in `live` mode it opens an EventSource over the
 * RoomBus and reconnects on drop. One hook, every realtime surface.
 */
export function useHarnessStream<T = unknown>(
  channel: string,
  options?: { mockEvery?: number; mockFactory?: (tick: number) => T },
) {
  const [events, setEvents] = useState<T[]>([]);
  const [connected, setConnected] = useState(false);
  const tick = useRef(0);

  useEffect(() => {
    if (HARNESS_SOURCE !== "live") {
      setConnected(true);
      const every = options?.mockEvery ?? 0;
      if (!every || !options?.mockFactory) return;
      const id = setInterval(() => {
        tick.current += 1;
        const factory = options.mockFactory!;
        setEvents((prev) => [...prev.slice(-50), factory(tick.current)]);
      }, every);
      return () => clearInterval(id);
    }

    const url = `${HARNESS_URL}/v1/stream/${encodeURIComponent(channel)}`;
    let source: EventSource | null = null;
    let retry: ReturnType<typeof setTimeout>;
    const open = () => {
      source = new EventSource(url);
      source.onopen = () => setConnected(true);
      source.onmessage = (e) => {
        try {
          setEvents((prev) => [...prev.slice(-50), JSON.parse(e.data) as T]);
        } catch {
          /* ignore malformed frame */
        }
      };
      source.onerror = () => {
        setConnected(false);
        source?.close();
        retry = setTimeout(open, 2000);
      };
    };
    open();
    return () => {
      source?.close();
      clearTimeout(retry);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [channel]);

  return { events, connected };
}
