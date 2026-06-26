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
    <section className={cn("cpw-block", className)}>
      <div className="cpw-block-header">
        <span className="cpw-block-eyebrow">{eyebrow ?? "block"}</span>
        <h2>{title}</h2>
        {action}
      </div>
      <div className="cpw-block-body">{children}</div>
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
                className="cpw-file-row"
                data-active={active ? "true" : "false"}
                style={{ "--cpw-depth": depth } as React.CSSProperties}
              >
                {depth === 0 ? (
                  <FolderOpen size={13} />
                ) : (
                  <FileCode2 size={13} />
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
            className="cpw-action-button"
            onClick={() => void host.emit({ kind: "run_agent", target: { id: patch.id, type: "patch" }, tier: "difficult" })}
          >
            <Bot size={13} /> <span className="cpw-action-copy">Review</span>
          </Button>
          <Button
            size="sm"
            variant="primary"
            className="cpw-action-button cpw-action-button-primary"
            onClick={() =>
              void host.emit({
                kind: "dispatch",
                job: { name: "applyPatch", args: { patch_id: patch.id } },
              })
            }
          >
            <CheckCircle2 size={13} /> <span className="cpw-action-copy">Apply</span>
          </Button>
        </div>
      }
    >
      <div className="cpw-diff-grid">
        {["before", "after"].map((side) => (
          <div key={side} className="cpw-diff-pane">
            <div className="cpw-diff-label">
              {side}
            </div>
            <pre className="cpw-code-surface">
              {stringProp(patch, side) ?? fallbackDiff(side)}
            </pre>
          </div>
        ))}
      </div>
      <div className="cpw-hunk-bar">
        <div className="cpw-badge-row">
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
          className="cpw-action-button"
          onClick={() => void host.emit({ kind: "run_agent", target: set.objects[0] ? { id: set.objects[0].id, type: set.objects[0].type } : { id: "workspace" }, tier: "simple" })}
        >
          <Play size={13} /> Run
        </Button>
      }
    >
      <div className="cpw-message-stack">
        {set.objects.map((message) => (
          <article key={message.id} className="cpw-message-card">
            <div className="cpw-message-meta">
              <Bot size={13} />
              <span>{stringProp(message, "role") ?? "assistant"}</span>
            </div>
            <p>{stringProp(message, "content")}</p>
          </article>
        ))}
      </div>
    </WorkspaceBlockFrame>
  );
}

export function RunTraceTimeline({ set }: ViewRenderProps) {
  return (
    <WorkspaceBlockFrame title="Run trace" eyebrow="RunTraceTimeline">
      <ol className="cpw-trace-list">
        <span aria-hidden className="cpw-trace-line" />
        {set.objects.map((step) => (
          <li key={step.id}>
            <span className="cpw-trace-node">
              <CircleDot size={9} />
            </span>
            <div className="cpw-trace-copy">
              <div className="cpw-trace-title">
                <span>{stringProp(step, "kind")}</span>
                <Badge tone={stringProp(step, "status") === "blocked" ? "warn" : "neutral"}>{stringProp(step, "status")}</Badge>
              </div>
              <p>{stringProp(step, "summary")}</p>
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
      <div className="cpw-tool-list">
        {set.objects.map((tool) => (
          <div key={tool.id} className="cpw-tool-row">
            <Wrench size={13} />
            <span>{stringProp(tool, "name")}</span>
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
      <div className="cpw-message-stack">
        {set.objects.map((artifact) => (
          <article key={artifact.id} className="cpw-context-card">
            <div className="cpw-message-meta">
              <Braces size={13} />
              <span>{stringProp(artifact, "title")}</span>
            </div>
            <p>{stringProp(artifact, "summary")}</p>
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
      <div className="cpw-terminal">
        <div className="cpw-terminal-command">
          <SquareTerminal size={13} />
          <span>$ {command}</span>
        </div>
        <pre>{output}</pre>
      </div>
    </WorkspaceBlockFrame>
  );
}

export function AgentRunBoard({ set }: ViewRenderProps) {
  const columns = ["queued", "running", "blocked", "done"];

  return (
    <WorkspaceBlockFrame title="Agent runs" eyebrow="AgentRunBoard">
      <div className="cpw-run-grid">
        {columns.map((status) => {
          const runs = set.objects.filter((run) => stringProp(run, "status") === status);
          return (
            <div key={status} className="cpw-run-column">
              <div className="cpw-run-column-head">
                <span>{status}</span>
                <strong>{runs.length}</strong>
              </div>
              <div className="cpw-run-card-list">
                {runs.map((run) => (
                  <article key={run.id} className="cpw-run-card">
                    <div className="cpw-run-title">
                      <TimerReset size={13} />
                      <span>{stringProp(run, "title")}</span>
                    </div>
                    <p>{stringProp(run, "summary")}</p>
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
      <pre className="cpw-editor-surface">
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
