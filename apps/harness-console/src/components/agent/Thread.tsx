"use client";

import * as React from "react";
import { Sparkles, User, ShieldCheck, ShieldAlert, ShieldX, Loader2, ChevronDown } from "lucide-react";
import type { ChatMessage, AlignmentVerdict } from "@/lib/harness";
import { Badge } from "@/components/ui/badge";
import { StatusDot } from "@/components/ui/status-dot";
import { cn, relativeTime } from "@/lib/utils";
import { RunTrace } from "./RunTrace";

/**
 * Thread: the conversation transcript. User turns sit right-aligned in an
 * oxblood-tinted bubble; assistant turns are a full-width document with the
 * head trace folded inline and the alignment-gate verdict pinned to the turn
 * when the run closes. A live (streaming) turn shows the trace building with a
 * working indicator instead of a final answer.
 */

const VERDICT: Record<
  AlignmentVerdict,
  { tone: "live" | "warn" | "accent" | "neutral"; icon: React.ComponentType<{ size?: number; className?: string }>; label: string }
> = {
  aligned: { tone: "live", icon: ShieldCheck, label: "aligned" },
  flagged: { tone: "warn", icon: ShieldAlert, label: "flagged" },
  blocked: { tone: "accent", icon: ShieldX, label: "blocked" },
  pending: { tone: "neutral", icon: ShieldCheck, label: "pending" },
};

function VerdictBadge({ verdict }: { verdict: AlignmentVerdict }) {
  const v = VERDICT[verdict];
  const Icon = v.icon;
  return (
    <Badge tone={v.tone} className="gap-1">
      <Icon size={11} />
      alignment: {v.label}
    </Badge>
  );
}

function UserTurn({ message }: { message: ChatMessage }) {
  return (
    <div className="flex justify-end">
      <div className="flex max-w-[80%] flex-col items-end gap-1">
        <div className="rounded-lg rounded-br-sm border border-[var(--ox)] bg-[var(--ox-tint)] px-3 py-2">
          <p className="whitespace-pre-wrap text-body text-ink">{message.content}</p>
        </div>
        <div className="flex items-center gap-1.5 font-mono text-[11px] text-faint">
          <User size={11} />
          you
          <span>·</span>
          {relativeTime(message.at)}
        </div>
      </div>
    </div>
  );
}

function AssistantTurn({ message, streaming }: { message: ChatMessage; streaming: boolean }) {
  const trace = message.trace ?? [];
  const [traceOpen, setTraceOpen] = React.useState(true);

  return (
    <div className="material p-4">
      <div className="mb-2 flex items-center gap-2">
        <span className="grid h-6 w-6 place-items-center rounded-full bg-ink text-bg">
          <Sparkles size={12} />
        </span>
        <span className="font-title text-subhead text-ink">Theorem agent</span>
        {streaming ? (
          <span className="flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground">
            <Loader2 size={12} className="animate-spin" />
            heads running
          </span>
        ) : (
          message.verdict && message.verdict !== "pending" && <VerdictBadge verdict={message.verdict} />
        )}
        <span className="ml-auto font-mono text-[11px] text-faint">{relativeTime(message.at)}</span>
      </div>

      {/* head + tool trace, folded inline */}
      {(trace.length > 0 || streaming) && (
        <div className="mb-3 rounded-md border border-line bg-bg">
          <button
            type="button"
            onClick={() => setTraceOpen((o) => !o)}
            className="flex w-full items-center gap-2 px-3 py-2 text-left"
          >
            <span className="rail-group-label">run trace</span>
            {streaming && <StatusDot status="live" pulse />}
            <span className="ml-auto font-mono text-[11px] text-faint">{trace.length} steps</span>
            <ChevronDown
              size={13}
              className={cn("text-muted-foreground transition-transform", traceOpen ? "rotate-180" : "")}
            />
          </button>
          {traceOpen && (
            <div className="border-t border-line px-3 py-2">
              <RunTrace entries={trace} live={streaming} />
            </div>
          )}
        </div>
      )}

      {streaming && !message.content ? (
        <div className="flex items-center gap-2 text-body text-muted-foreground">
          <span className="h-2 w-2 animate-[pulse_1.2s_ease-in-out_infinite] rounded-full bg-faint" />
          composing the peers&rsquo; answer&hellip;
        </div>
      ) : (
        <p className="whitespace-pre-wrap text-body text-ink">{message.content}</p>
      )}

      {/* which heads participated, derived from the trace */}
      {!streaming && trace.some((t) => t.role === "head") && (
        <div className="mt-3 flex flex-wrap items-center gap-1.5 border-t border-line pt-3">
          <span className="rail-group-label mr-1">heads</span>
          {Array.from(new Set(trace.filter((t) => t.role === "head").map((t) => t.head))).map((h) => (
            <Badge key={h} tone="neutral" className="gap-1">
              <StatusDot status="ok" />
              {h}
            </Badge>
          ))}
        </div>
      )}
    </div>
  );
}

export function Thread({
  messages,
  streamingId,
  className,
}: {
  messages: ChatMessage[];
  streamingId?: string | null;
  className?: string;
}) {
  const endRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [messages.length, streamingId]);

  return (
    <div className={cn("flex flex-col gap-4", className)}>
      {messages.map((m) =>
        m.role === "user" ? (
          <UserTurn key={m.id} message={m} />
        ) : (
          <AssistantTurn key={m.id} message={m} streaming={m.id === streamingId} />
        ),
      )}
      <div ref={endRef} />
    </div>
  );
}
