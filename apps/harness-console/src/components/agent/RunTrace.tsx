"use client";

import * as React from "react";
import { Brain, Wrench, Eye, GitBranch, ShieldCheck, Circle } from "lucide-react";
import type { TraceEntry, ChatRole } from "@/lib/harness";
import { StatusDot } from "@/components/ui/status-dot";
import { cn, relativeTime } from "@/lib/utils";

/**
 * RunTrace: the ordered ledger of what the heads did inside a single turn. Each
 * trace entry is a head contribution, a tool call, an observation, or the
 * alignment-gate system note. The vertical rail makes it read as a transcript
 * of the run, not a list. When the run is live, the last entry pulses.
 */

function roleIcon(role: ChatRole) {
  switch (role) {
    case "head":
      return Brain;
    case "tool":
      return Wrench;
    case "system":
      return ShieldCheck;
    case "assistant":
      return GitBranch;
    default:
      return Eye;
  }
}

function roleLabel(e: TraceEntry): string {
  if (e.role === "head") return e.head ?? "head";
  if (e.role === "tool") return e.tool ?? "tool";
  if (e.role === "system") return "gate";
  return e.role;
}

export function RunTrace({
  entries,
  live = false,
  className,
}: {
  entries: TraceEntry[];
  live?: boolean;
  className?: string;
}) {
  if (entries.length === 0) {
    return (
      <div className={cn("flex items-center gap-2 px-1 py-2 text-label text-faint", className)}>
        <Circle size={12} />
        <span className="font-mono">no trace yet</span>
      </div>
    );
  }

  return (
    <ol className={cn("relative flex flex-col gap-0", className)}>
      {/* the connecting rail */}
      <span aria-hidden className="absolute bottom-2 left-[7px] top-2 w-px bg-line" />
      {entries.map((e, i) => {
        const Icon = roleIcon(e.role);
        const isLast = i === entries.length - 1;
        const pulsing = live && isLast;
        return (
          <li key={e.id} className="relative flex gap-3 py-1.5 pl-0">
            <span
              className={cn(
                "relative z-[1] mt-0.5 grid h-[15px] w-[15px] shrink-0 place-items-center rounded-full border border-line bg-bg",
                e.role === "system" && "border-[var(--live)] text-live",
                e.role === "head" && "text-ink",
                e.role === "tool" && "text-muted-foreground",
              )}
            >
              <Icon size={9} />
            </span>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span
                  className={cn(
                    "font-mono text-[11px] uppercase tracking-wide",
                    e.role === "head" ? "text-ink" : "text-muted-foreground",
                  )}
                >
                  {roleLabel(e)}
                </span>
                {pulsing && <StatusDot status="live" pulse />}
                <span className="ml-auto font-mono text-[11px] text-faint">{relativeTime(e.at)}</span>
              </div>
              <p className="mt-0.5 text-label text-muted-foreground">{e.content}</p>
            </div>
          </li>
        );
      })}
    </ol>
  );
}
