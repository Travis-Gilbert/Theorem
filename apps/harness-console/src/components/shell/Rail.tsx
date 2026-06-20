"use client";

import { ChevronDown } from "lucide-react";
import { SpaceTree } from "./SpaceTree";
import { UsagePulse } from "./UsagePulse";
import { useSpaceTypes } from "@/lib/spaces/useSpaceTypes";

export function Rail({ horizontal = false }: { horizontal?: boolean }) {
  const { spaces, rename, reorder } = useSpaceTypes();

  if (horizontal) {
    return <SpaceTree spaces={spaces} horizontal onRename={rename} onReorder={reorder} />;
  }

  return (
    <aside
      className="rail-shell z-20 hidden h-full shrink-0 flex-col border-r border-line bg-surface md:flex"
      style={{ width: "var(--rail-w)" }}
    >
      <div className="flex items-center gap-2 px-3 py-3">
        <div className="grid h-7 w-7 place-items-center rounded-md bg-ox font-mono text-[13px] font-bold text-white">
          H
        </div>
        <span className="font-title text-subhead text-ink">Harness</span>
      </div>

      {/* tenant selector */}
      <button className="mx-3 mb-2 flex items-center justify-between rounded-md border border-line bg-bg px-2.5 py-1.5 font-mono text-label text-ink hover:bg-surface-2">
        <span className="truncate">{process.env.NEXT_PUBLIC_DEFAULT_TENANT ?? "default"}</span>
        <ChevronDown size={13} className="text-muted-foreground" />
      </button>

      <SpaceTree spaces={spaces} onRename={rename} onReorder={reorder} />
      <UsagePulse />
    </aside>
  );
}
