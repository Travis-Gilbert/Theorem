"use client";

import * as React from "react";
import { CreditCard, Gauge, Receipt, Wallet } from "lucide-react";
import type { UsagePeriod } from "@/lib/harness";
import { harness } from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";
import { usePageToc } from "@/components/island/useScrollSpy";
import { PageHeader, Section } from "@/components/common/PageHeader";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch, Label, Separator } from "@/components/ui/misc";
import { Skeleton } from "@/components/ui/misc";
import { EmptyState } from "@/components/common/EmptyState";
import { toast } from "@/components/ui/toaster";
import { UsageMeter, UsageBars } from "@/components/usage/UsageMeter";

export default function UsagePage() {
  usePageToc();
  const { data, loading, error } = useAsync(() => harness.getUsage(), []);

  // Pay-per-use is a local toggle layered over the loaded period until a live
  // billing endpoint exists; it persists for the session. Seed it from the
  // loaded data during render when that data first arrives (no mirror effect).
  const [payPerUse, setPayPerUse] = React.useState(false);
  const [seen, setSeen] = React.useState<UsagePeriod | null>(null);
  if (data && data !== seen) {
    setSeen(data);
    setPayPerUse(data.payPerUse);
  }

  return (
    <div className="mx-auto max-w-4xl">
      <PageHeader
        eyebrow="Metering"
        title="Usage"
        description="Per-tenant metering and pay-per-use. Requests and tool calls are counted against your plan limit this period; cross the line and the console prompts an upgrade rather than failing a run."
      />

      {loading && <UsageSkeleton />}

      {!loading && error && (
        <EmptyState icon={Gauge} title="Could not load usage" description={error} />
      )}

      {!loading && !error && data && (
        <Loaded usage={data} payPerUse={payPerUse} setPayPerUse={setPayPerUse} />
      )}
    </div>
  );
}

function Loaded({
  usage,
  payPerUse,
  setPayPerUse,
}: {
  usage: UsagePeriod;
  payPerUse: boolean;
  setPayPerUse: (v: boolean) => void;
}) {
  function togglePayPerUse(on: boolean) {
    setPayPerUse(on);
    toast.success(
      on
        ? "Pay-per-use is on: runs continue past the plan limit, billed per call."
        : "Pay-per-use is off: runs stop at the plan limit.",
    );
  }

  return (
    <>
      <Section id="current-period" title="Current period">
        <UsageMeter usage={usage} />
      </Section>

      <Section id="activity" title="Activity">
        <Card calm>
          <CardContent className="pt-4">
            <UsageBars series={usage.series} />
          </CardContent>
        </Card>
      </Section>

      <Section id="plan-and-billing" title="Plan & billing">
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          {/* Plan + limits */}
          <Card calm>
            <CardContent className="flex flex-col gap-4 pt-4">
              <Row icon={Gauge} label="plan">
                <Badge tone={usage.plan === "Free" ? "neutral" : "accent"}>{usage.plan}</Badge>
              </Row>
              <Separator />
              <Row icon={Receipt} label="request limit">
                <span className="font-mono text-label text-ink tabular-nums">
                  {usage.limit.toLocaleString()} / period
                </span>
              </Row>
              <Row icon={Wallet} label="period">
                <span className="font-mono text-[11px] text-muted-foreground">{usage.periodLabel}</span>
              </Row>

              <Separator />

              {/* Pay-per-use toggle, Mistral-style metered billing. */}
              <div className="flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <Label className="text-ink">pay-per-use</Label>
                  <p className="mt-0.5 font-mono text-[11px] text-muted-foreground">
                    Keep running past the plan limit; each request and tool call is metered and
                    billed.
                  </p>
                </div>
                <Switch
                  checked={payPerUse}
                  onCheckedChange={togglePayPerUse}
                  aria-label="Toggle pay-per-use billing"
                />
              </div>
            </CardContent>
          </Card>

          {/* Billing / payment */}
          <Card calm>
            <CardContent className="flex flex-col gap-4 pt-4">
              <Row icon={CreditCard} label="payment method">
                <span className="font-mono text-[11px] text-muted-foreground">
                  {usage.plan === "Free" ? "none on file" : "card on file"}
                </span>
              </Row>
              <p className="text-label text-muted-foreground">
                {usage.plan === "Free"
                  ? "On the free tier nothing is charged. Add a payment method to enable pay-per-use or upgrade."
                  : "Metered charges settle at the end of the period."}
              </p>

              <div className="flex flex-wrap items-center gap-2">
                <Button variant="primary" size="sm">
                  {usage.plan === "Free" ? "Add payment method" : "Manage billing"}
                </Button>
                <Button variant="outline" size="sm">
                  View invoices
                </Button>
              </div>

              <Separator />

              {/* USDC top-up: noted possibility, not yet available. */}
              <div className="flex items-start justify-between gap-3 rounded-md border border-dashed border-line bg-bg p-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-label text-muted-foreground">USDC top-up</span>
                    <Badge tone="neutral">coming soon</Badge>
                  </div>
                  <p className="mt-0.5 font-mono text-[11px] text-faint">
                    Pre-fund metered usage with stablecoin credits.
                  </p>
                </div>
                <Button variant="subtle" size="sm" disabled>
                  Top up
                </Button>
              </div>
            </CardContent>
          </Card>
        </div>
      </Section>
    </>
  );
}

function Row({
  icon: Icon,
  label,
  children,
}: {
  icon: React.ComponentType<{ size?: number; className?: string }>;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="flex items-center gap-2 text-muted-foreground">
        <Icon size={15} />
        <span className="rail-group-label">{label}</span>
      </div>
      {children}
    </div>
  );
}

function UsageSkeleton() {
  return (
    <div className="flex flex-col gap-8">
      <Skeleton className="h-56 w-full rounded-lg" />
      <Skeleton className="h-48 w-full rounded-lg" />
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <Skeleton className="h-56 w-full rounded-lg" />
        <Skeleton className="h-56 w-full rounded-lg" />
      </div>
    </div>
  );
}
