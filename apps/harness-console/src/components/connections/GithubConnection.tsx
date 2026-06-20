"use client";

import * as React from "react";
import { Github, GitBranch, ShieldCheck, RefreshCw, AlertTriangle } from "lucide-react";
import { harness, type ConnectedRepo, type IngestStatus } from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { StatusDot } from "@/components/ui/status-dot";
import { Skeleton } from "@/components/ui/misc";
import { EmptyState } from "@/components/common/EmptyState";
import { relativeTime, cn } from "@/lib/utils";

/**
 * GitHub connection panel. When disconnected, a single primary "Connect GitHub"
 * affordance. When connected, the account plus the list of repos the harness
 * ingests into the code graph, each with its ingestion status (StatusDot +
 * Badge), symbol count, and last-updated time, plus an "authorize
 * repositories" affordance.
 */

// Map each ingest status to the status-dot vocabulary + a badge tone, so the
// glyph and the label always agree.
const STATUS: Record<
  IngestStatus,
  { dot: "live" | "idle" | "ok" | "error"; tone: BadgeProps["tone"]; pulse: boolean; label: string }
> = {
  queued: { dot: "idle", tone: "neutral", pulse: false, label: "queued" },
  ingesting: { dot: "live", tone: "live", pulse: true, label: "ingesting" },
  indexed: { dot: "ok", tone: "accent", pulse: false, label: "indexed" },
  error: { dot: "error", tone: "warn", pulse: false, label: "error" },
};

function fmtSymbols(n?: number): string {
  if (n == null) return "--";
  return n.toLocaleString();
}

function RepoRow({ repo }: { repo: ConnectedRepo }) {
  const s = STATUS[repo.status];
  return (
    <div className="flex items-center justify-between gap-4 px-4 py-3">
      <div className="flex min-w-0 items-center gap-3">
        <GitBranch size={15} className="shrink-0 text-faint" />
        <div className="min-w-0">
          <div className="truncate font-mono text-label text-ink">
            {repo.owner}/<span className="text-ink">{repo.name}</span>
          </div>
          <div className="mt-0.5 flex items-center gap-2 font-mono text-[11px] text-faint">
            <span>{fmtSymbols(repo.symbols)} symbols</span>
            <span aria-hidden>&middot;</span>
            <span>updated {relativeTime(repo.updated)}</span>
          </div>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <StatusDot status={s.dot} pulse={s.pulse} />
        <Badge tone={s.tone}>{s.label}</Badge>
      </div>
    </div>
  );
}

export function GithubConnection() {
  const { data, loading, error, reload } = useAsync(() => harness.getGithub(), []);

  if (loading) {
    return (
      <Card calm>
        <CardHeader>
          <Skeleton className="h-5 w-40" />
          <Skeleton className="mt-1 h-4 w-64" />
        </CardHeader>
        <CardContent className="space-y-3">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-12 w-full" />
          ))}
        </CardContent>
      </Card>
    );
  }

  if (error) {
    return (
      <EmptyState
        icon={AlertTriangle}
        title="Could not load the GitHub connection"
        description={error}
        action={
          <Button variant="outline" size="sm" onClick={reload}>
            <RefreshCw size={14} /> Retry
          </Button>
        }
      />
    );
  }

  const github = data;

  // Disconnected: one primary affordance.
  if (!github || !github.connected) {
    return (
      <Card calm className="overflow-hidden">
        <div className="flex flex-col items-center gap-4 px-6 py-12 text-center">
          <div className="grid h-12 w-12 place-items-center rounded-full bg-surface-2 text-ink">
            <Github size={22} />
          </div>
          <div>
            <p className="font-title text-subhead text-ink">Connect GitHub</p>
            <p className="mt-1 max-w-md text-label text-muted-foreground">
              Authorize the harness to read selected repositories. It ingests each one into the code graph so agents
              can search, explain, and explore your code structurally.
            </p>
          </div>
          <Button variant="primary" size="md" onClick={reload}>
            <Github size={16} /> Connect GitHub
          </Button>
        </div>
      </Card>
    );
  }

  return (
    <Card calm className="overflow-hidden">
      <CardHeader className="flex-row items-center justify-between gap-4 space-y-0">
        <div className="flex items-center gap-3">
          <div className="grid h-10 w-10 place-items-center rounded-full bg-surface-2 text-ink">
            <Github size={18} />
          </div>
          <div>
            <CardTitle className="text-body">
              {github.account}
            </CardTitle>
            <CardDescription className="flex items-center gap-1.5">
              <ShieldCheck size={12} className="text-[var(--live)]" />
              Connected. Selected repositories ingest into the code graph.
            </CardDescription>
          </div>
        </div>
        <Button variant="outline" size="sm" onClick={reload}>
          <ShieldCheck size={14} /> Authorize repositories
        </Button>
      </CardHeader>

      <CardContent className="px-0 pb-0">
        {github.repos.length === 0 ? (
          <div className="px-4 pb-4">
            <EmptyState
              icon={GitBranch}
              title="No repositories authorized yet"
              description="Authorize a repository to ingest it into the code graph."
              action={
                <Button variant="primary" size="sm" onClick={reload}>
                  <ShieldCheck size={14} /> Authorize repositories
                </Button>
              }
            />
          </div>
        ) : (
          <div className={cn("border-t border-line")}>
            <div className="flex items-center justify-between px-4 py-2">
              <span className="rail-group-label">Connected repositories</span>
              <span className="font-mono text-[11px] text-faint">{github.repos.length} repos</span>
            </div>
            <div className="divide-y divide-line border-t border-line">
              {github.repos.map((repo) => (
                <RepoRow key={repo.id} repo={repo} />
              ))}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
