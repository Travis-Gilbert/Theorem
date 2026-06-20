"use client";

import * as React from "react";
import { ListTodo } from "lucide-react";
import { type Task, type TaskState, type TaskPriority } from "@/lib/harness";
import { cn, relativeTime } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from "@/components/ui/select";
import { EmptyState } from "@/components/common/EmptyState";

const COLUMNS: { state: TaskState; label: string }[] = [
  { state: "queued", label: "Queued" },
  { state: "running", label: "Running" },
  { state: "blocked", label: "Blocked" },
  { state: "done", label: "Done" },
];

const PRIORITY_TONE: Record<TaskPriority, "neutral" | "accent" | "warn"> = {
  high: "accent",
  normal: "neutral",
  low: "neutral",
};

/** Dispatch-v2 jobs as a board. Each card can be moved between states; the move
 *  calls updateTaskState (the THG job board verbs in production). */
export function TaskBoard({ tasks, onMove }: { tasks: Task[]; onMove: (id: string, state: TaskState) => void }) {
  if (!tasks.length) {
    return <EmptyState icon={ListTodo} title="No tasks" description="Submitted jobs and follow-ups will show up here." />;
  }
  return (
    <div className="grid h-full grid-cols-1 gap-3 overflow-y-auto p-4 md:grid-cols-2 xl:grid-cols-4">
      {COLUMNS.map((col) => {
        const items = tasks.filter((t) => t.state === col.state);
        return (
          <div key={col.state} className="flex min-h-0 flex-col gap-2">
            <div className="flex items-center justify-between px-1">
              <span className="rail-group-label">{col.label}</span>
              <span className="font-mono text-[11px] text-faint">{items.length}</span>
            </div>
            <div className="flex flex-col gap-2">
              {items.map((t) => (
                <div key={t.id} className="material flex flex-col gap-2 p-3">
                  <p className="text-body leading-snug text-ink">{t.title}</p>
                  <div className="flex flex-wrap items-center gap-1.5">
                    <Badge tone={PRIORITY_TONE[t.priority]}>{t.priority}</Badge>
                    {t.targetHead && <Badge tone="neutral">{t.targetHead}</Badge>}
                    <span className="ml-auto font-mono text-[10px] text-faint">{relativeTime(t.updated)}</span>
                  </div>
                  {t.note && <p className="font-mono text-[10px] text-muted-foreground">{t.note}</p>}
                  <Select value={t.state} onValueChange={(v) => onMove(t.id, v as TaskState)}>
                    <SelectTrigger className="h-7">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {COLUMNS.map((c) => (
                        <SelectItem key={c.state} value={c.state}>
                          {c.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              ))}
              {!items.length && (
                <div className={cn("rounded-md border border-dashed border-line px-3 py-6 text-center font-mono text-[10px] text-faint")}>
                  empty
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
