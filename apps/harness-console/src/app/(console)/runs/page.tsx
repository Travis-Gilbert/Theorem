"use client";

import * as React from "react";
import { harness } from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";
import { usePageToc } from "@/components/island/useScrollSpy";
import { PageHeader, Section } from "@/components/common/PageHeader";
import { RunList } from "@/components/runs/RunList";
import { RunDetail } from "@/components/runs/RunDetail";

/**
 * Runs: read-only run history plus replay. Master-detail: the run rail on the
 * left, the ordered event ledger with a replay control on the right (stacked on
 * narrow). Selecting a run loads its ledger; replay walks the timeline.
 */
export default function RunsPage() {
  usePageToc();

  const { data: runs, loading: runsLoading } = useAsync(() => harness.listRuns(), []);
  const [picked, setPicked] = React.useState<string | null>(null);

  // The effective selection: an explicit pick, else default to the first run
  // once the list loads. Derived (no effect-driven setState) so selection never
  // lags the data, and a stale pick falls back gracefully.
  const selectedId =
    (picked && runs?.some((r) => r.id === picked) ? picked : null) ?? runs?.[0]?.id ?? null;

  const { data: run, loading: runLoading } = useAsync(
    () => (selectedId ? harness.getRun(selectedId) : Promise.resolve(null)),
    [selectedId],
  );

  return (
    <div>
      <PageHeader
        eyebrow="history"
        title="Runs"
        description="The composed-agent run ledger. Open a run to read its ordered event timeline and replay it step by step."
      />

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[340px_minmax(0,1fr)]">
        <Section id="runs-list" title="History" className="mb-0">
          <RunList runs={runs} loading={runsLoading} selectedId={selectedId} onSelect={setPicked} />
        </Section>

        <Section id="runs-detail" title="Ledger" className="mb-0">
          {/* Keyed by run id so switching runs remounts the detail and resets the
              replay cursor/timer to a clean state. */}
          <RunDetail key={selectedId ?? "none"} run={run} loading={Boolean(selectedId) && runLoading} />
        </Section>
      </div>
    </div>
  );
}
