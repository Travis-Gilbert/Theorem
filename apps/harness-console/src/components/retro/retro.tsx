import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * RetroUI components retokenized to the console palette (Phase 3). Adopt the
 * structural boldness (hard border, offset shadow), drop the neon. Used for the
 * instrument-brutalist surfaces: the collaborative IDE frame and the MCP hub.
 */

export function RetroFrame({
  className,
  accent,
  children,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & { accent?: boolean }) {
  return (
    <div className={cn("retro-frame", accent && "retro-frame-accent", className)} {...props}>
      {children}
    </div>
  );
}

/** The Glowing Shadow card: one hero accent, an animated oxblood glow border.
 *  Use sparingly, on a single element per surface. */
export function GlowCard({ className, children, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cn("glow-border", className)} {...props}>
      <div className="relative h-full w-full rounded-[10px] bg-surface p-4">{children}</div>
    </div>
  );
}

/** The animated wave footer, a motion flourish. */
export function WaveFooter({ className }: { className?: string }) {
  return (
    <div className={cn("wave-footer pointer-events-none overflow-hidden", className)} aria-hidden>
      <svg viewBox="0 0 1200 60" width="200%" height="60" preserveAspectRatio="none">
        <path
          d="M0 30 C 150 0, 300 60, 450 30 S 750 0, 900 30 S 1200 60, 1200 30 L1200 60 L0 60 Z"
          fill="var(--ox-tint)"
        />
        <path
          d="M0 36 C 150 12, 300 60, 450 36 S 750 12, 900 36 S 1200 60, 1200 36"
          fill="none"
          stroke="var(--ox)"
          strokeOpacity="0.4"
          strokeWidth="1.5"
        />
      </svg>
    </div>
  );
}
