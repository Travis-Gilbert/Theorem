"use client";

import * as React from "react";
import { cn, relativeTime } from "@/lib/utils";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { StatusDot } from "@/components/ui/status-dot";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/misc";
import { EmptyState } from "@/components/common/EmptyState";
import { Play, Pause, RotateCcw, ListOrdered, ScrollText } from "lucide-react";
import type { Run, RunStep, RunStatus, AlignmentVerdict } from "@/lib/harness";

/**
 * The run detail pane: an ordered event ledger plus a replay control. Replay
 * walks the timeline one step at a time on a timer, highlighting the current
 * step and scrolling it into view; pause holds the cursor, reset returns to the
 * start. When not replaying, every step is shown in full.
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

const STEP_MS = 900;

function DetailSkeleton() {
  return (
    <div className="flex flex-col gap-4" aria-busy="true">
      <Skeleton className="h-24 w-full rounded-lg" />
      <Skeleton className="h-[420px] w-full rounded-lg" />
    </div>
  );
}

function LedgerRow({
  step,
  state,
}: {
  step: RunStep;
  state: "past" | "current" | "future";
}) {
  return (
    <li
      data-step={step.index}
      className={cn(
        "flex gap-3 border-b border-line/60 px-4 py-3 transition-colors last:border-b-0",
        state === "current" && "bg-[var(--ox-tint)]",
        state === "future" && "opacity-45",
      )}
    >
      <div className="flex w-8 shrink-0 flex-col items-center pt-0.5">
        <span
          className={cn(
            "grid h-6 w-6 place-items-center rounded-full font-mono text-[11px]",
            state === "current"
              ? "bg-ox text-white"
              : state === "past"
                ? "bg-surface-2 text-ink"
                : "bg-surface-2 text-faint",
          )}
        >
          {step.index}
        </span>
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-2">
          <div className="flex min-w-0 items-center gap-2">
            <Badge tone="neutral">{step.kind}</Badge>
            {step.actor && <span className="font-mono text-[11px] text-muted-foreground">{step.actor}</span>}
          </div>
          <span className="shrink-0 font-mono text-[11px] text-faint">{relativeTime(step.at)}</span>
        </div>
        <p className="mt-1 text-body text-ink">{step.summary}</p>
        {step.detail && <p className="mt-0.5 font-mono text-[11px] text-faint">{step.detail}</p>}
      </div>
    </li>
  );
}

export function RunDetail({ run, loading }: { run: Run | null; loading: boolean }) {
  // Replay cursor: -1 means not started / reset. `playing` advances it on a timer.
  const [cursor, setCursor] = React.useState(-1);
  const [playing, setPlaying] = React.useState(false);
  const viewportRef = React.useRef<HTMLDivElement | null>(null);

  const steps = React.useMemo<RunStep[]>(
    () => (run ? [...run.steps].sort((a, b) => a.index - b.index) : []),
    [run],
  );

  // The replay timer: advance the cursor while playing. State writes happen in
  // the timer callback (never synchronously in the effect body), so the timeline
  // walks one step at a time and halts itself once the last step is reached.
  React.useEffect(() => {
    if (!playing) return;
    const id = setTimeout(() => {
      setCursor((c) => {
        const next = c + 1;
        if (next >= steps.length - 1) setPlaying(false);
        return Math.min(next, steps.length - 1);
      });
    }, STEP_MS);
    return () => clearTimeout(id);
  }, [playing, cursor, steps.length]);

  // Scroll the active step into view as replay walks the timeline.
  React.useEffect(() => {
    if (cursor < 0) return;
    const el = viewportRef.current?.querySelector<HTMLElement>(`[data-step="${cursor}"]`);
    el?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [cursor]);

  if (loading) return <DetailSkeleton />;

  if (!run) {
    return (
      <EmptyState
        icon={ScrollText}
        title="Select a run"
        description="Pick a run from the rail to open its ordered event ledger and replay the timeline."
      />
    );
  }

  const atEnd = cursor >= steps.length - 1;
  const replaying = cursor >= 0 || playing;

  const onPlayPause = () => {
    if (playing) {
      setPlaying(false);
      return;
    }
    // Starting from a reset or finished state begins at the first step.
    if (cursor < 0 || atEnd) setCursor(0);
    setPlaying(true);
  };

  const onReset = () => {
    setPlaying(false);
    setCursor(-1);
  };

  const stepState = (index: number): "past" | "current" | "future" => {
    if (!replaying) return "past"; // not replaying: show all in full
    if (index < cursor) return "past";
    if (index === cursor) return "current";
    return "future";
  };

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <CardTitle>{run.goal}</CardTitle>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span className="inline-flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground">
                  <StatusDot status={STATUS_DOT[run.status]} pulse={run.status === "running"} />
                  {run.status}
                </span>
                <Badge tone={VERDICT_TONE[run.verdict]}>{run.verdict}</Badge>
                <span className="font-mono text-[11px] text-faint">{run.stepCount} steps</span>
                <span className="font-mono text-[11px] text-faint">started {relativeTime(run.started)}</span>
                {run.finished && (
                  <span className="font-mono text-[11px] text-faint">finished {relativeTime(run.finished)}</span>
                )}
              </div>
            </div>
          </div>
        </CardHeader>
        <CardContent>
          <div className="flex items-center gap-2">
            <Button variant="primary" size="sm" onClick={onPlayPause} disabled={steps.length === 0}>
              {playing ? <Pause size={14} /> : <Play size={14} />}
              {playing ? "Pause" : atEnd && cursor >= 0 ? "Replay" : cursor < 0 ? "Replay" : "Resume"}
            </Button>
            <Button variant="outline" size="sm" onClick={onReset} disabled={cursor < 0 && !playing}>
              <RotateCcw size={14} />
              Reset
            </Button>
            <div className="ml-auto font-mono text-[11px] text-faint">
              {replaying ? `step ${Math.max(0, cursor) + 1} / ${steps.length}` : `${steps.length} steps`}
            </div>
          </div>
          {/* Progress bar tracks the replay cursor across the ledger. */}
          <div className="mt-3 h-1 w-full overflow-hidden rounded-full bg-surface-2">
            <div
              className="h-full bg-ox transition-[width] duration-300 ease-out"
              style={{ width: replaying ? `${((Math.max(0, cursor) + 1) / steps.length) * 100}%` : "0%" }}
            />
          </div>
        </CardContent>
      </Card>

      <Card calm>
        <div className="flex items-center gap-2 border-b border-line px-4 py-3 font-mono text-label text-ink">
          <ListOrdered size={14} className="text-muted-foreground" />
          event ledger
        </div>
        {steps.length === 0 ? (
          <div className="px-4 py-10 text-center text-label text-muted-foreground">This run recorded no steps.</div>
        ) : (
          <div ref={viewportRef} className="max-h-[440px] overflow-y-auto">
            <ul>
              {steps.map((step) => (
                <LedgerRow key={step.index} step={step} state={stepState(step.index)} />
              ))}
            </ul>
          </div>
        )}
      </Card>
    </div>
  );
}
