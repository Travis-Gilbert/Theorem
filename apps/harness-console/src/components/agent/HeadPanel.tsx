"use client";

import * as React from "react";
import Link from "next/link";
import { Cpu, Database, Hash, KeyRound, ArrowRight } from "lucide-react";
import type { AgentBinding, Head } from "@/lib/harness";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { StatusDot } from "@/components/ui/status-dot";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/misc";
import { cn } from "@/lib/utils";

/**
 * HeadPanel: the composed agent's makeup, in the right rail. It is the answer to
 * "what am I actually talking to" -- the heads (model + provider + role), each
 * head's provider-key health (green when ok, amber when missing/invalid, with
 * the exact path to /providers), and the scope (memory scopes + rooms) the turn
 * runs against. When every head is missing a key, the panel itself becomes the
 * call to action rather than an inert list.
 */

function keyDot(status: Head["keyStatus"]) {
  if (status === "ok") return <StatusDot status="ok" />;
  return <StatusDot status="warn" />;
}

function HeadRow({ head }: { head: Head }) {
  const needsKey = head.keyStatus !== "ok";
  return (
    <div className="rounded-md border border-line bg-bg p-3">
      <div className="flex items-center gap-2">
        <span className="grid h-6 w-6 place-items-center rounded bg-surface-2 text-muted-foreground">
          <Cpu size={13} />
        </span>
        <span className="font-mono text-label font-medium text-ink">{head.id}</span>
        <span className="ml-auto flex items-center gap-1.5">
          {keyDot(head.keyStatus)}
          <span
            className={cn(
              "font-mono text-[11px]",
              head.keyStatus === "ok" ? "text-live" : "text-[var(--warn)]",
            )}
          >
            {head.keyStatus === "ok" ? "key ok" : `key ${head.keyStatus}`}
          </span>
        </span>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        <Badge tone="neutral">{head.model}</Badge>
        <Badge tone="ink">{head.provider}</Badge>
      </div>
      <p className="mt-2 font-mono text-[11px] text-faint">{head.role}</p>
      {needsKey && (
        <Link
          href={`/providers?provider=${head.provider}`}
          className="mt-2 inline-flex items-center gap-1 font-mono text-[11px] text-ox underline-offset-4 hover:underline"
        >
          <KeyRound size={11} />
          add the {head.provider} key
          <ArrowRight size={11} />
        </Link>
      )}
    </div>
  );
}

function ScopeBlock({ binding }: { binding: AgentBinding }) {
  return (
    <div className="space-y-3">
      <div>
        <div className="rail-group-label mb-1.5 flex items-center gap-1.5">
          <Database size={11} /> memory scopes
        </div>
        <div className="flex flex-wrap gap-1.5">
          {binding.scope.memoryScopes.map((s) => (
            <Badge key={s} tone="neutral">
              {s}
            </Badge>
          ))}
        </div>
      </div>
      <div>
        <div className="rail-group-label mb-1.5 flex items-center gap-1.5">
          <Hash size={11} /> rooms
        </div>
        <div className="flex flex-wrap gap-1.5">
          {binding.scope.rooms.map((r) => (
            <Badge key={r} tone="neutral">
              {r}
            </Badge>
          ))}
        </div>
      </div>
    </div>
  );
}

export function HeadPanelSkeleton() {
  return (
    <div className="space-y-4">
      <Card calm>
        <CardHeader>
          <Skeleton className="h-4 w-28" />
        </CardHeader>
        <CardContent className="space-y-2">
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-20 w-full" />
        </CardContent>
      </Card>
    </div>
  );
}

export function HeadPanel({ binding, className }: { binding: AgentBinding; className?: string }) {
  const live = binding.heads.filter((h) => h.keyStatus === "ok");
  const allMissing = live.length === 0;

  return (
    <div className={cn("space-y-4", className)}>
      <Card calm>
        <CardHeader className="flex-row items-center justify-between">
          <CardTitle className="text-body">Composed agent</CardTitle>
          <Badge tone={allMissing ? "warn" : "live"}>
            {binding.bindingId}
          </Badge>
        </CardHeader>
        <CardContent className="space-y-2 pt-0">
          <div className="mb-1 flex items-center justify-between font-mono text-[11px] text-muted-foreground">
            <span>{binding.heads.length} heads as peers</span>
            <span className={allMissing ? "text-[var(--warn)]" : "text-live"}>
              {live.length}/{binding.heads.length} keyed
            </span>
          </div>
          {binding.heads.map((h) => (
            <HeadRow key={h.id} head={h} />
          ))}
        </CardContent>
      </Card>

      <Card calm>
        <CardHeader>
          <CardTitle className="text-body">Scope</CardTitle>
        </CardHeader>
        <CardContent className="pt-0">
          <ScopeBlock binding={binding} />
        </CardContent>
      </Card>

      {allMissing && (
        <Card calm className="border-[var(--warn)]">
          <CardContent className="space-y-2 p-4">
            <p className="text-label text-ink">
              No provider keys are configured, so no head can run.
            </p>
            <Button asChild variant="primary" size="sm" className="w-full">
              <Link href="/providers">
                <KeyRound size={13} /> Add a provider key
              </Link>
            </Button>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
