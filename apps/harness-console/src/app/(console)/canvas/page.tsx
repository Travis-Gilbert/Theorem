"use client";

import dynamic from "next/dynamic";

// React Flow needs the DOM; load it client-only to avoid SSR measurement work.
const Canvas = dynamic(() => import("@/components/canvas/Canvas").then((m) => m.Canvas), {
  ssr: false,
  loading: () => (
    <div className="grid h-full place-items-center font-mono text-label text-muted-foreground">
      Loading canvas...
    </div>
  ),
});

export default function CanvasPage() {
  return <Canvas />;
}
