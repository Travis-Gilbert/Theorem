"use client";

import { harness } from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";

/** The Mistral usage-widget slot, repurposed: requests this period, keys, plan.
 *  Pinned at the bottom of the rail. */
export function UsagePulse() {
  const { data: usage } = useAsync(() => harness.getUsage());
  const { data: keys } = useAsync(() => harness.listKeys());
  const pct = usage ? Math.min(100, Math.round((usage.requests / usage.limit) * 100)) : 0;
  return (
    <div className="border-t border-line px-3 py-3">
      <div className="mb-1.5 flex items-center justify-between font-mono text-[11px] text-muted-foreground">
        <span>{(usage?.requests ?? 0).toLocaleString()} req</span>
        <span>
          {keys?.length ?? 0} keys &middot; {usage?.plan ?? "Free"}
        </span>
      </div>
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-2">
        <div className="h-full rounded-full bg-ox transition-all" style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}
