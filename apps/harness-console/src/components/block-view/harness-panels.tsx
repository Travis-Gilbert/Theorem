"use client";

import * as React from "react";
import {
  Bot,
  Braces,
  CheckCircle2,
  CircleDot,
  FileCode2,
  FolderOpen,
  GitPullRequest,
  Play,
  SquareTerminal,
  TimerReset,
  Wrench,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { JsonValue, ObjectRef, ViewRenderProps } from "@/lib/block-view";

export function WorkspaceBlockFrame({
  title,
  eyebrow,
  action,
  className,
  children,
}: {
  title: string;
  eyebrow?: string;
  action?: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <section className={cn("flex min-h-0 flex-col overflow-hidden rounded-lg border border-line bg-surface", className)}>
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-line px-3">
        <span className="rail-group-label">{eyebrow ?? "block"}</span>
        <h2 className="min-w-0 flex-1 truncate font-title text-subhead text-ink">{title}</h2>
        {action}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">{children}</div>
    </section>
  );
}

export function FileTreePanel({ set, host }: ViewRenderProps) {
  const files = set.objects.filter((object) => object.type === "file");

  return (
    <WorkspaceBlockFrame title="Files" eyebrow="FileTreePanel">
      <ul className="p-2">
        {files.map((file) => {
          const path = stringProp(file, "path") ?? file.id;
          const depth = path.split("/").length - 1;
          const active = stringProp(file, "active") === "true";
          return (
            <li key={file.id}>
              <button
                type="button"
                onClick={() => void host.emit({ kind: "open", id: file.id, view: "code-editor" })}
                className={cn(
                  "flex min-h-8 w-full items-center gap-1.5 rounded-md py-1 pr-2 text-left font-mono text-label transition-colors",
                  active ? "bg-ox-tint text-ox" : "text-ink hover:bg-surface-2",
                )}
                style={{ paddingLeft: `calc(var(--space-2) + ${depth} * var(--space-4))` }}
              >
                {depth === 0 ? (
                  <FolderOpen size={13} className="shrink-0 text-faint" />
                ) : (
                  <FileCode2 size={13} className="shrink-0 text-faint" />
                )}
                <span className="truncate">{path.split("/").pop()}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </WorkspaceBlockFrame>
  );
}

export function PatchReviewPanel({ set, host }: ViewRenderProps) {
  const patch = set.objects.find((object) => object.type === "patch") ?? set.objects[0];

  if (!patch) {
    return (
      <WorkspaceBlockFrame title="Patch review" eyebrow="PatchReviewPanel">
        <div className="p-3 text-label text-muted-foreground">No patch object available.</div>
      </WorkspaceBlockFrame>
    );
  }

  const hunks = arrayProp(patch, "hunks");

  return (
    <WorkspaceBlockFrame
      title={stringProp(patch, "title") ?? "Patch review"}
      eyebrow="PatchReviewPanel"
      action={
        <div className="flex items-center gap-1.5">
          <Button
            size="sm"
            variant="outline"
            onClick={() => void host.emit({ kind: "run_agent", target: { id: patch.id, type: "patch" }, tier: "difficult" })}
          >
            <Bot size={13} /> Review
          </Button>
          <Button
            size="sm"
            variant="primary"
            onClick={() =>
              void host.emit({
                kind: "dispatch",
                job: { name: "applyPatch", args: { patch_id: patch.id } },
              })
            }
          >
            <CheckCircle2 size={13} /> Apply
          </Button>
        </div>
      }
    >
      <div className="grid h-full min-h-0 grid-cols-1 overflow-hidden lg:grid-cols-2">
        {["before", "after"].map((side) => (
          <div key={side} className="min-h-0 overflow-auto border-b border-line lg:border-b-0 lg:border-r">
            <div className="sticky top-0 border-b border-line bg-bg px-3 py-2 font-mono text-label text-muted-foreground">
              {side}
            </div>
            <pre className="min-h-full whitespace-pre-wrap p-3 font-mono text-label leading-relaxed text-ink">
              {stringProp(patch, side) ?? fallbackDiff(side)}
            </pre>
          </div>
        ))}
      </div>
      <div className="border-t border-line bg-bg px-3 py-2">
        <div className="flex flex-wrap gap-1.5">
          {hunks.map((hunk) => (
            <Badge key={String(hunk)} tone="neutral">
              {String(hunk)}
            </Badge>
          ))}
        </div>
      </div>
    </WorkspaceBlockFrame>
  );
}

export function AgentThreadPanel({ set, host }: ViewRenderProps) {
  return (
    <WorkspaceBlockFrame
      title="Agent thread"
      eyebrow="AgentThreadPanel"
      action={
        <Button
          size="sm"
          variant="outline"
          onClick={() => void host.emit({ kind: "run_agent", target: set.objects[0] ? { id: set.objects[0].id, type: set.objects[0].type } : { id: "workspace" }, tier: "simple" })}
        >
          <Play size={13} /> Run
        </Button>
      }
    >
      <div className="space-y-3 p-3">
        {set.objects.map((message) => (
          <article key={message.id} className="rounded-md border border-line bg-bg p-3">
            <div className="mb-1 flex items-center gap-2">
              <Bot size={13} className="text-ox" />
              <span className="font-mono text-label text-muted-foreground">{stringProp(message, "role") ?? "assistant"}</span>
            </div>
            <p className="text-body leading-relaxed text-ink">{stringProp(message, "content")}</p>
          </article>
        ))}
      </div>
    </WorkspaceBlockFrame>
  );
}

export function RunTraceTimeline({ set }: ViewRenderProps) {
  return (
    <WorkspaceBlockFrame title="Run trace" eyebrow="RunTraceTimeline">
      <ol className="relative p-3">
        <span aria-hidden className="absolute bottom-4 left-[19px] top-4 w-px bg-line" />
        {set.objects.map((step) => (
          <li key={step.id} className="relative flex gap-3 py-2">
            <span className="relative z-[1] mt-1 grid h-4 w-4 place-items-center rounded-full border border-line bg-bg text-ox">
              <CircleDot size={9} />
            </span>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="font-mono text-label text-ink">{stringProp(step, "kind")}</span>
                <Badge tone={stringProp(step, "status") === "blocked" ? "warn" : "neutral"}>{stringProp(step, "status")}</Badge>
              </div>
              <p className="mt-1 text-label leading-relaxed text-muted-foreground">{stringProp(step, "summary")}</p>
            </div>
          </li>
        ))}
      </ol>
    </WorkspaceBlockFrame>
  );
}

export function ToolActivityPanel({ set }: ViewRenderProps) {
  return (
    <WorkspaceBlockFrame title="Tools" eyebrow="ToolActivityPanel">
      <div className="grid gap-2 p-3">
        {set.objects.map((tool) => (
          <div key={tool.id} className="flex items-center gap-2 rounded-md border border-line bg-bg px-3 py-2">
            <Wrench size={13} className="text-faint" />
            <span className="min-w-0 flex-1 truncate font-mono text-label text-ink">{stringProp(tool, "name")}</span>
            <Badge tone={stringProp(tool, "status") === "ok" ? "live" : "neutral"}>{stringProp(tool, "status")}</Badge>
          </div>
        ))}
      </div>
    </WorkspaceBlockFrame>
  );
}

export function ContextArtifactDrawer({ set }: ViewRenderProps) {
  return (
    <WorkspaceBlockFrame title="Context" eyebrow="ContextArtifactDrawer">
      <div className="space-y-3 p-3">
        {set.objects.map((artifact) => (
          <article key={artifact.id} className="rounded-md border border-line bg-bg p-3">
            <div className="mb-2 flex items-center gap-2">
              <Braces size={13} className="text-faint" />
              <span className="font-mono text-label text-ink">{stringProp(artifact, "title")}</span>
            </div>
            <p className="text-label leading-relaxed text-muted-foreground">{stringProp(artifact, "summary")}</p>
          </article>
        ))}
      </div>
    </WorkspaceBlockFrame>
  );
}

export function TerminalPanel({ set }: ViewRenderProps) {
  const command = stringProp(set.objects[0], "command") ?? "npm run lint";
  const output = stringProp(set.objects[0], "output") ?? "ready";

  return (
    <WorkspaceBlockFrame title="Terminal" eyebrow="TerminalPanel">
      <div className="h-full bg-ink p-3 font-mono text-label text-bg">
        <div className="mb-2 flex items-center gap-2 text-bg/80">
          <SquareTerminal size={13} />
          <span>$ {command}</span>
        </div>
        <pre className="whitespace-pre-wrap leading-relaxed">{output}</pre>
      </div>
    </WorkspaceBlockFrame>
  );
}

export function AgentRunBoard({ set }: ViewRenderProps) {
  const columns = ["queued", "running", "blocked", "done"];

  return (
    <WorkspaceBlockFrame title="Agent runs" eyebrow="AgentRunBoard">
      <div className="grid h-full grid-cols-1 gap-3 p-3 md:grid-cols-4">
        {columns.map((status) => {
          const runs = set.objects.filter((run) => stringProp(run, "status") === status);
          return (
            <div key={status} className="min-h-0 rounded-md border border-line bg-bg">
              <div className="flex items-center justify-between border-b border-line px-3 py-2">
                <span className="rail-group-label">{status}</span>
                <span className="font-mono text-label text-faint">{runs.length}</span>
              </div>
              <div className="space-y-2 p-2">
                {runs.map((run) => (
                  <article key={run.id} className="rounded-md border border-line bg-surface p-2">
                    <div className="mb-1 flex items-center gap-2">
                      <TimerReset size={13} className="text-faint" />
                      <span className="font-mono text-label text-ink">{stringProp(run, "title")}</span>
                    </div>
                    <p className="text-label text-muted-foreground">{stringProp(run, "summary")}</p>
                  </article>
                ))}
              </div>
            </div>
          );
        })}
      </div>
    </WorkspaceBlockFrame>
  );
}

export function CodeEditorPanel({ set }: ViewRenderProps) {
  const file = set.objects.find((object) => stringProp(object, "active") === "true") ?? set.objects[0];

  return (
    <WorkspaceBlockFrame
      title={stringProp(file, "path") ?? "Editor"}
      eyebrow="CodeMirrorPanel"
      action={
        <Badge tone="neutral">
          <GitPullRequest size={11} /> object block
        </Badge>
      }
    >
      <pre className="min-h-full overflow-auto bg-bg p-4 font-mono text-label leading-relaxed text-ink">
        {stringProp(file, "content") ?? ""}
      </pre>
    </WorkspaceBlockFrame>
  );
}

function stringProp(object: ObjectRef | undefined, key: string): string | undefined {
  const value = object?.properties[key];
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return undefined;
}

function arrayProp(object: ObjectRef | undefined, key: string): readonly JsonValue[] {
  const value = object?.properties[key];
  return Array.isArray(value) ? value : [];
}

function fallbackDiff(side: string): string {
  return side === "before"
    ? "function renderPatch(patch) {\n  return patch.diff;\n}"
    : "function renderPatch(patch) {\n  return viewFor(patch.shape).render(patch);\n}";
}
