"use client";

import { cn, relativeTime } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { StatusDot } from "@/components/ui/status-dot";
import { Skeleton } from "@/components/ui/misc";
import { EmptyState } from "@/components/common/EmptyState";
import { History } from "lucide-react";
import type { Run, RunStatus, AlignmentVerdict } from "@/lib/harness";

/**
 * The run rail: run history as a selectable list. Each row shows the goal, a
 * status dot + chip, the alignment verdict, and the step count. Selecting a row
 * drives the detail ledger.
 */

const STATUS_DOT: Record<RunStatus, "live" | "ok" | "error" | "idle"> = {
  running: "live",
  complete: "ok",
  error: "error",
  cancelled: "idle",
};

const VERDICT_TONE: Record<AlignmentVerdict, "live" | "warn" | "accent" | "neutral"> = {
  aligned: "live",
  flagged: "warn",
  blocked: "accent",
  pending: "neutral",
};

export function RunList({
  runs,
  loading,
  selectedId,
  onSelect,
}: {
  runs: Run[] | null;
  loading: boolean;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  if (loading) {
    return (
      <div className="flex flex-col gap-2" aria-busy="true">
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton key={i} className="h-[88px] w-full rounded-lg" />
        ))}
      </div>
    );
  }

  if (!runs || runs.length === 0) {
    return (
      <EmptyState
        icon={History}
        title="No runs yet"
        description="Composed-agent runs land here as an ordered event ledger you can open and replay."
      />
    );
  }

  return (
    <ul className="flex flex-col gap-2" role="listbox" aria-label="Run history">
      {runs.map((run) => {
        const active = run.id === selectedId;
        return (
          <li key={run.id}>
            <button
              type="button"
              role="option"
              aria-selected={active}
              onClick={() => onSelect(run.id)}
              className={cn(
                "group w-full rounded-lg border px-3 py-3 text-left transition-colors",
                active
                  ? "border-[var(--ox)] bg-[var(--ox-tint)]"
                  : "border-line bg-surface hover:bg-surface-2",
              )}
            >
              <div className="flex items-start justify-between gap-2">
                <p className="line-clamp-2 min-w-0 text-body text-ink">{run.goal}</p>
                <span className="shrink-0 font-mono text-[11px] text-faint">{relativeTime(run.started)}</span>
              </div>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span className="inline-flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground">
                  <StatusDot status={STATUS_DOT[run.status]} pulse={run.status === "running"} />
                  {run.status}
                </span>
                <Badge tone={VERDICT_TONE[run.verdict]}>{run.verdict}</Badge>
                <span className="font-mono text-[11px] text-faint">{run.stepCount} steps</span>
              </div>
            </button>
          </li>
        );
      })}
    </ul>
  );
}
