"use client";

import { Coins, KeyRound, ShieldCheck } from "lucide-react";
import { harness } from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";
import { usePageToc } from "@/components/island/useScrollSpy";
import { PageHeader, Section } from "@/components/common/PageHeader";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/misc";
import { EmptyState } from "@/components/common/EmptyState";
import { ProviderList } from "@/components/providers/ProviderList";

export default function ProvidersPage() {
  usePageToc();
  const { data, loading, error, reload } = useAsync(() => harness.listProviders(), []);

  const providers = data ?? [];
  const ready = providers.filter((p) => p.keyStatus === "ok").length;
  const byok = providers.filter((p) => p.mode === "byok").length;
  const credits = providers.length - byok;

  return (
    <div className="mx-auto max-w-4xl">
      <PageHeader
        eyebrow="Outbound"
        title="Providers"
        description="The model provider keys the composed agent's heads run on. Each key is resolved at run time as a credential reference; the raw secret never lives in the pack registry."
      />

      <Section id="provider-keys" title="Provider keys">
        {/* Summary strip: how many providers are runnable right now. */}
        <div className="mb-4 grid grid-cols-1 gap-3 sm:grid-cols-3">
          <SummaryStat icon={ShieldCheck} label="runnable" value={`${ready} / ${providers.length}`} hint="keys validated" />
          <SummaryStat icon={KeyRound} label="bring-your-own-key" value={String(byok)} hint="resolve your secret" />
          <SummaryStat icon={Coins} label="harness credits" value={String(credits)} hint="billed per use" />
        </div>

        {loading && (
          <div className="flex flex-col gap-3">
            {[0, 1, 2, 3].map((i) => (
              <Skeleton key={i} className="h-[68px] w-full rounded-lg" />
            ))}
          </div>
        )}

        {!loading && error && (
          <EmptyState
            icon={KeyRound}
            title="Could not load providers"
            description={error}
          />
        )}

        {!loading && !error && (
          <ProviderList providers={providers} onChanged={reload} />
        )}
      </Section>

      <Section id="how-keys-resolve" title="How keys resolve">
        <Card calm>
          <CardContent className="pt-4">
            <ol className="flex flex-col gap-3 text-body text-muted-foreground">
              <ResolveStep n={1} title="You store a secret">
                The console sends it to the harness and keeps only a{" "}
                <span className="font-mono text-ink">credential_ref</span>. The secret is never read
                back into the browser.
              </ResolveStep>
              <ResolveStep n={2} title="A head requests a provider">
                When the composed agent runs a head bound to a provider, the harness resolves the
                reference to the live secret for that one call.
              </ResolveStep>
              <ResolveStep n={3} title="Validation reports inline">
                Validate checks the key against the provider and reports success or failure here, so a
                bad key surfaces before a run fails opaquely.
              </ResolveStep>
            </ol>
            <p className="mt-4 border-t border-line pt-4 font-mono text-[11px] text-faint">
              BYOK resolves your own key. Harness credits bill metered usage instead.{" "}
              <Badge tone="neutral">credential_ref</Badge> material stays out of the pack registry.
            </p>
          </CardContent>
        </Card>
      </Section>
    </div>
  );
}

function SummaryStat({
  icon: Icon,
  label,
  value,
  hint,
}: {
  icon: React.ComponentType<{ size?: number; className?: string }>;
  label: string;
  value: string;
  hint: string;
}) {
  return (
    <Card calm className="flex items-center gap-3 p-4">
      <div className="grid h-9 w-9 shrink-0 place-items-center rounded-md bg-surface-2 text-muted-foreground">
        <Icon size={16} />
      </div>
      <div className="min-w-0">
        <div className="rail-group-label">{label}</div>
        <div className="font-title text-subhead text-ink">{value}</div>
        <div className="font-mono text-[11px] text-faint">{hint}</div>
      </div>
    </Card>
  );
}

function ResolveStep({ n, title, children }: { n: number; title: string; children: React.ReactNode }) {
  return (
    <li className="flex items-start gap-3">
      <span className="grid h-6 w-6 shrink-0 place-items-center rounded-full border border-line font-mono text-[11px] text-ox">
        {n}
      </span>
      <span>
        <span className="font-mono text-label uppercase tracking-wide text-ink">{title}</span>{" "}
        <span className="text-label">{children}</span>
      </span>
    </li>
  );
}
