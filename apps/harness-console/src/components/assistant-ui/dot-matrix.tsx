"use client";

import * as React from "react";
import { cn } from "@/lib/utils";

export const dotMatrixStates = [
  "idle",
  "loading",
  "thinking",
  "streaming",
  "searching",
  "syncing",
  "connecting",
  "waiting",
  "uploading",
  "downloading",
  "listening",
  "speaking",
  "recording",
  "success",
  "error",
  "warning",
  "info",
  "paused",
  "stopped",
  "offline",
] as const;

export type DotMatrixState = (typeof dotMatrixStates)[number];

const ACTIVE_DOTS: Record<DotMatrixState, readonly number[]> = {
  idle: [6, 8, 12, 16, 18],
  loading: [1, 3, 7, 11, 12, 13, 17, 21, 23],
  thinking: [0, 6, 12, 18, 24],
  streaming: [2, 7, 12, 17, 22, 4, 9, 14],
  searching: [10, 11, 12, 13, 14],
  syncing: [2, 3, 4, 9, 14, 19, 24, 23, 22, 17, 12],
  connecting: [7, 11, 12, 13, 17],
  waiting: [11, 12, 13],
  uploading: [2, 6, 7, 8, 12, 17, 22],
  downloading: [2, 7, 12, 16, 17, 18, 22],
  listening: [5, 10, 15, 6, 11, 16, 12, 17, 22],
  speaking: [0, 5, 10, 15, 20, 6, 11, 16, 7, 12, 17, 22],
  recording: [12],
  success: [4, 8, 12, 16, 20],
  error: [0, 4, 6, 8, 12, 16, 18, 20, 24],
  warning: [2, 7, 12, 17, 22],
  info: [2, 12, 17, 22],
  paused: [6, 11, 16, 8, 13, 18],
  stopped: [6, 7, 8, 11, 12, 13, 16, 17, 18],
  offline: [12],
};

const TONE_CLASS: Partial<Record<DotMatrixState, string>> = {
  success: "cp-dot-matrix-success",
  error: "cp-dot-matrix-error",
  warning: "cp-dot-matrix-warning",
  info: "cp-dot-matrix-info",
  recording: "cp-dot-matrix-error",
  offline: "cp-dot-matrix-offline",
  stopped: "cp-dot-matrix-offline",
  paused: "cp-dot-matrix-warning",
};

export function DotMatrix({
  state = "loading",
  label,
  className,
}: {
  state?: DotMatrixState;
  label?: string;
  className?: string;
}) {
  const active = ACTIVE_DOTS[state];

  return (
    <span
      className={cn("cp-dot-matrix", TONE_CLASS[state], className)}
      data-slot="dot-matrix"
      data-state={state}
      role="status"
      aria-live="polite"
    >
      <span className="sr-only">{label ?? state}</span>
      <svg viewBox="0 0 28 28" aria-hidden="true" focusable="false">
        {Array.from({ length: 25 }, (_, index) => {
          const x = 4 + (index % 5) * 5;
          const y = 4 + Math.floor(index / 5) * 5;
          const isActive = active.includes(index);
          const style = {
            "--dm-on": isActive ? "0.92" : "0.28",
            "--dm-off": isActive ? "0.36" : "0.1",
            "--dm-delay": `${(index * 73) % 610}ms`,
            "--dm-duration": `${900 + ((index * 97) % 560)}ms`,
          } as React.CSSProperties;

          return (
            <circle
              key={index}
              data-slot="dot-matrix-dot"
              data-active={isActive ? "true" : "false"}
              cx={x}
              cy={y}
              r="1.55"
              style={style}
            />
          );
        })}
      </svg>
    </span>
  );
}
