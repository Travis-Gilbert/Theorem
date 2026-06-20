import { cn } from "@/lib/utils";

type Status = "live" | "idle" | "away" | "ok" | "error" | "warn";

const COLOR: Record<Status, string> = {
  live: "var(--live)",
  ok: "var(--live)",
  idle: "var(--warn)",
  warn: "var(--warn)",
  away: "var(--faint)",
  error: "var(--ox)",
};

/** The status-dot vocabulary, reused by Rooms presence and provider/key health. */
export function StatusDot({ status, pulse, className }: { status: Status; pulse?: boolean; className?: string }) {
  return (
    <span
      className={cn("status-dot", pulse && status === "live" && "animate-[pulse_2s_ease-in-out_infinite]", className)}
      style={{ background: COLOR[status] }}
      aria-label={status}
    />
  );
}
