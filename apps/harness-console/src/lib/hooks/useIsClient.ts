"use client";

import { useSyncExternalStore } from "react";

const emptySubscribe = () => () => {};

/**
 * True only after client hydration, false on the server. This is the idiomatic
 * React 18/19 way to gate client-only widgets (CodeMirror, React Flow) without a
 * `setState`-in-effect mount guard: `useSyncExternalStore` returns the server
 * snapshot (false) during SSR/hydration and the client snapshot (true) after,
 * with no cascading render.
 */
export function useIsClient(): boolean {
  return useSyncExternalStore(
    emptySubscribe,
    () => true,
    () => false,
  );
}
