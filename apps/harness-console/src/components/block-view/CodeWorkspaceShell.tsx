"use client";

import * as React from "react";
import { AlertTriangle } from "lucide-react";
import { HARNESS_OBJECT_SETS, mockBlockHost } from "@/lib/block-view/harness-fixtures";
import { HARNESS_VIEW_DESCRIPTORS } from "@/lib/block-view/harness-registry";
import { ViewRegistry } from "@/lib/block-view";
import { Badge } from "@/components/ui/badge";
import type { ObjectSet, ViewDescriptor } from "@/lib/block-view";

const registry = new ViewRegistry(HARNESS_VIEW_DESCRIPTORS);

export function CodeWorkspaceShell() {
  return (
    <div className="flex h-full min-h-0 flex-col bg-bg">
      <div className="flex h-12 shrink-0 items-center gap-3 border-b border-line px-4">
        <div className="min-w-0 flex-1">
          <div className="rail-group-label">CodeWorkspaceShell</div>
          <h1 className="truncate font-title text-subhead text-ink">Harness blocks over RustyRed objects</h1>
        </div>
        <Badge tone="neutral">object/view contract</Badge>
        <Badge tone="live">tokenized</Badge>
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-1 gap-3 p-3 lg:grid-cols-[240px_minmax(0,1fr)_340px]">
        <div className="grid min-h-0 grid-rows-[minmax(0,1fr)_220px] gap-3">
          <BlockSlot viewId="file-tree" set={HARNESS_OBJECT_SETS.files} />
          <BlockSlot viewId="terminal" set={HARNESS_OBJECT_SETS.terminal} />
        </div>

        <div className="grid min-h-0 grid-rows-[minmax(0,1fr)_220px] gap-3">
          <div className="grid min-h-0 grid-cols-1 gap-3 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
            <BlockSlot viewId="code-editor" set={HARNESS_OBJECT_SETS.files} />
            <BlockSlot viewId="patch-review" set={HARNESS_OBJECT_SETS.patch} />
          </div>
          <BlockSlot viewId="agent-run-board" set={HARNESS_OBJECT_SETS.runs} />
        </div>

        <div className="grid min-h-0 grid-rows-[minmax(0,1fr)_220px_220px] gap-3">
          <BlockSlot viewId="agent-thread" set={HARNESS_OBJECT_SETS.thread} />
          <BlockSlot viewId="run-trace" set={HARNESS_OBJECT_SETS.trace} />
          <div className="grid min-h-0 grid-cols-2 gap-3">
            <BlockSlot viewId="tool-activity" set={HARNESS_OBJECT_SETS.tools} />
            <BlockSlot viewId="context-artifact" set={HARNESS_OBJECT_SETS.context} />
          </div>
        </div>
      </div>
    </div>
  );
}

function BlockSlot({ viewId, set }: { viewId: string; set: ObjectSet }) {
  const view = React.useMemo(() => pickView(viewId, set), [set, viewId]);

  if (!view) {
    return (
      <div className="flex min-h-0 items-center justify-center rounded-lg border border-dashed border-line bg-surface p-4 text-label text-muted-foreground">
        <AlertTriangle size={14} className="mr-2" />
        No descriptor matched {viewId}
      </div>
    );
  }

  const View = view.render;
  return <View set={set} host={mockBlockHost} />;
}

function pickView(viewId: string, set: ObjectSet): ViewDescriptor | undefined {
  return registry.viewsFor(set.shape).find((view) => view.id === viewId);
}
