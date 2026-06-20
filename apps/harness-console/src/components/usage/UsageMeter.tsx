"use client";

import * as React from "react";
import { ArrowUpRight, Wrench, Zap } from "lucide-react";
import type { UsagePeriod } from "@/lib/harness";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** When usage crosses this fraction of the limit, we surface an upgrade CTA
 *  instead of letting a run fail opaquely at the cap. */
const UPGRADE_THRESHOLD = 0.8;

export function UsageMeter({ usage }: { usage: UsagePeriod }) {
  const ratio = usage.limit > 0 ? usage.requests / usage.limit : 0;
  const pct = Math.min(100, Math.round(ratio * 100));
  const over = usage.requests >= usage.limit;
  const near = ratio >= UPGRADE_THRESHOLD;

  const barTone = over ? "bg-ox" : near ? "bg-[var(--warn)]" : "bg-[var(--live)]";

  return (
    <Card calm>
      <CardContent className="pt-4">
        {/* Headline: requests vs limit with the plan + period. */}
        <div className="mb-3 flex flex-wrap items-end justify-between gap-2">
          <div>
            <div className="rail-group-label">requests this period</div>
            <div className="flex items-baseline gap-2">
              <span className="font-title text-display text-ink tabular-nums">
                {usage.requests.toLocaleString()}
              </span>
              <span className="font-mono text-label text-muted-foreground">/ {usage.limit.toLocaleString()}</span>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Badge tone={usage.plan === "Free" ? "neutral" : "accent"}>{usage.plan} plan</Badge>
            <span className="font-mono text-[11px] text-faint">{usage.periodLabel}</span>
          </div>
        </div>

        {/* Progress bar, recolored by headroom. */}
        <div
          className="h-2.5 w-full overflow-hidden rounded-full bg-surface-2"
          role="progressbar"
          aria-valuenow={pct}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-label="Requests used this period"
        >
          <div
            className={cn("h-full rounded-full transition-[width] duration-500", barTone)}
            style={{ width: `${Math.max(2, pct)}%` }}
          />
        </div>
        <div className="mt-1.5 flex items-center justify-between font-mono text-[11px] text-muted-foreground">
          <span>{pct}% used</span>
          <span>
            {over
              ? "limit reached"
              : `${(usage.limit - usage.requests).toLocaleString()} requests left`}
          </span>
        </div>

        {/* Upgrade prompt: appears as headroom runs low, before failures. */}
        {(near || over) && (
          <div
            className={cn(
              "mt-4 flex flex-wrap items-center justify-between gap-3 rounded-md border p-3",
              over ? "border-[var(--ox)] bg-[var(--ox-tint)]" : "border-[var(--warn)] bg-surface-2",
            )}
          >
            <div className="min-w-0">
              <p className={cn("font-mono text-label", over ? "text-ox" : "text-[var(--warn)]")}>
                {over
                  ? "You have reached the free-tier request limit."
                  : "You are close to the free-tier request limit."}
              </p>
              <p className="mt-0.5 font-mono text-[11px] text-muted-foreground">
                Upgrade or enable pay-per-use so runs keep going instead of failing at the cap.
              </p>
            </div>
            <Button variant="primary" size="sm">
              <ArrowUpRight size={14} />
              Upgrade plan
            </Button>
          </div>
        )}

        {/* Secondary meters. Tool calls are the real metered signal. */}
        <div className="mt-5 grid grid-cols-1 gap-3 sm:grid-cols-2">
          <Tile icon={Zap} label="requests" value={usage.requests.toLocaleString()} />
          <Tile icon={Wrench} label="tool calls" value={usage.toolCalls.toLocaleString()} />
        </div>
      </CardContent>
    </Card>
  );
}

function Tile({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ size?: number; className?: string }>;
  label: string;
  value: string;
}) {
  return (
    <div className="flex items-center gap-3 rounded-md border border-line bg-bg p-3">
      <div className="grid h-8 w-8 shrink-0 place-items-center rounded bg-surface-2 text-muted-foreground">
        <Icon size={15} />
      </div>
      <div>
        <div className="rail-group-label">{label}</div>
        <div className="font-title text-subhead text-ink tabular-nums">{value}</div>
      </div>
    </div>
  );
}

/** A token-only bar chart of the daily series, drawn with flex divs. No chart
 *  library: the bars are <div>s scaled to the period max. */
export function UsageBars({ series }: { series: UsagePeriod["series"] }) {
  const max = React.useMemo(() => Math.max(1, ...series.map((d) => d.value)), [series]);
  const [hover, setHover] = React.useState<number | null>(null);

  if (series.length === 0) {
    return <p className="font-mono text-label text-faint">No daily activity recorded this period.</p>;
  }

  const peak = series.reduce((a, b) => (b.value > a.value ? b : a), series[0]);

  return (
    <div>
      <div className="flex items-baseline justify-between">
        <div className="rail-group-label">daily tool calls</div>
        <div className="font-mono text-[11px] text-muted-foreground">
          {hover != null ? (
            <>
              <span className="text-ink">{series[hover].value.toLocaleString()}</span> on{" "}
              {series[hover].label}
            </>
          ) : (
            <>
              peak <span className="text-ink">{peak.value.toLocaleString()}</span> on {peak.label}
            </>
          )}
        </div>
      </div>

      <div className="mt-3 flex h-32 items-end gap-1" role="img" aria-label="Daily tool calls this period">
        {series.map((d, i) => {
          const h = Math.max(3, Math.round((d.value / max) * 100));
          return (
            <div
              key={`${d.label}-${i}`}
              className="group relative flex flex-1 items-end"
              style={{ height: "100%" }}
              onMouseEnter={() => setHover(i)}
              onMouseLeave={() => setHover((cur) => (cur === i ? null : cur))}
            >
              <div
                className={cn(
                  "w-full rounded-t-sm transition-colors",
                  hover === i ? "bg-ox" : "bg-[var(--live)]/70 group-hover:bg-ox",
                )}
                style={{ height: `${h}%`, background: hover === i ? "var(--ox)" : undefined }}
                title={`${d.label}: ${d.value.toLocaleString()}`}
              />
            </div>
          );
        })}
      </div>

      <div className="mt-1.5 flex justify-between font-mono text-[11px] text-faint">
        <span>{series[0].label}</span>
        <span>{series[series.length - 1].label}</span>
      </div>
    </div>
  );
}
