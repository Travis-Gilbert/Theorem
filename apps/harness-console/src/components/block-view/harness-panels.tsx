"use client";

import * as React from "react";
import { AuiProvider, ExternalThread, useAui, type ExternalThreadMessage } from "@assistant-ui/react";
import { DndContext, useDraggable } from "@dnd-kit/core";
import { Renderer } from "@openuidev/react-lang";
import { createColumnHelper, flexRender, getCoreRowModel, useReactTable } from "@tanstack/react-table";
import type { CSSProperties } from "react";
import { Tree, type NodeRendererProps } from "react-arborist";
import CodeMirror from "@uiw/react-codemirror";
import CodeMirrorMerge from "react-codemirror-merge";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  Bot,
  Braces,
  CheckCircle2,
  CircleDot,
  ChevronRight,
  FileCode2,
  FolderOpen,
  Play,
  Send,
  SquareTerminal,
  TimerReset,
  Wrench,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { JsonValue, ObjectRef, ViewRenderProps } from "@/lib/block-view";
import {
  commonplaceCodeExtensions,
  commonplaceCodeMirrorTheme,
  readOnlyExtensions,
  type CommonPlaceCodeLanguage,
} from "./commonplace-code-theme";
import { commonplaceOpenUiLibrary, sceneArtifactOpenUiResponse } from "./commonplace-openui";

const MergeOriginal = CodeMirrorMerge.Original;
const MergeModified = CodeMirrorMerge.Modified;

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
  const data = React.useMemo(() => buildFileTree(files), [files]);
  const activeId = files.find((file) => stringProp(file, "active") === "true")?.id;
  const height = Math.max(150, Math.min(340, countTreeNodes(data) * 30 + 12));

  return (
    <WorkspaceBlockFrame title="Files" eyebrow="Explorer">
      <Tree<FileTreeNode>
        data={data}
        width="100%"
        height={height}
        rowHeight={30}
        indent={14}
        idAccessor="id"
        childrenAccessor="children"
        openByDefault
        disableDrag
        disableDrop
        disableEdit
        disableMultiSelection
        selection={activeId}
        onActivate={(node) => {
          if (!node.data.objectId || node.data.kind === "directory") return;
          void host.emit({ kind: "open", id: node.data.objectId, view: "code-editor" });
        }}
        className="cpw-arborist-tree"
      >
        {FileTreeNode}
      </Tree>
    </WorkspaceBlockFrame>
  );
}

export function PatchReviewPanel({ set, host }: ViewRenderProps) {
  const patch = set.objects.find((object) => object.type === "patch") ?? set.objects[0];

  if (!patch) {
    return (
      <WorkspaceBlockFrame title="Patch review" eyebrow="Review">
        <div className="p-3 text-label text-muted-foreground">No patch object available.</div>
      </WorkspaceBlockFrame>
    );
  }

  const hunks = arrayProp(patch, "hunks");
  const before = stringProp(patch, "before") ?? fallbackDiff("before");
  const after = stringProp(patch, "after") ?? fallbackDiff("after");

  return (
    <WorkspaceBlockFrame
      title={stringProp(patch, "title") ?? "Patch review"}
      eyebrow="Review"
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
      <div className="cpw-merge-shell">
        <CodeMirrorMerge
          theme={commonplaceCodeMirrorTheme}
          orientation="a-b"
          highlightChanges
          gutter
          collapseUnchanged={{ margin: 2, minSize: 4 }}
          className="cpw-merge-view"
        >
          <MergeOriginal
            value={before}
            extensions={[...commonplaceCodeExtensions("typescript"), ...readOnlyExtensions]}
          />
          <MergeModified
            value={after}
            extensions={[...commonplaceCodeExtensions("typescript"), ...readOnlyExtensions]}
          />
        </CodeMirrorMerge>
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
  const messages = React.useMemo(() => toExternalThreadMessages(set.objects), [set.objects]);
  const aui = useAui({
    thread: ExternalThread({
      messages,
      isRunning: false,
      onNew: (message) => {
        const content = "content" in message ? message.content : [];
        const text = Array.isArray(content)
          ? content.map((part) => ("text" in part ? part.text : "")).join("\n")
          : "";
        void host.emit({
          kind: "run_agent",
          target: { id: text.trim() || "workspace" },
          tier: "simple",
        });
      },
    }),
  });

  return (
    <WorkspaceBlockFrame
      title="Agent thread"
      eyebrow="Agent"
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
      <AuiProvider value={aui}>
        <div className="cpw-message-stack" data-source="@assistant-ui/react ExternalThread">
          {messages.map((message) => (
            <article key={message.id} className="cpw-message-card" data-role={message.role}>
              <div className="cpw-message-meta">
                <Bot size={13} />
                <span>{message.role}</span>
              </div>
              <p>{message.content.map((part) => (part.type === "text" ? part.text : "")).join("\n")}</p>
            </article>
          ))}
        </div>
        <form
          className="cpw-assistant-composer"
          onSubmit={(event) => {
            event.preventDefault();
            const form = event.currentTarget;
            const formData = new FormData(form);
            const prompt = String(formData.get("prompt") ?? "").trim();
            if (!prompt) return;
            void host.emit({ kind: "run_agent", target: { id: prompt }, tier: "simple" });
            form.reset();
          }}
        >
          <textarea name="prompt" placeholder="Ask the Theorem agent" className="cpw-assistant-input" rows={1} />
          <button className="cpw-assistant-send" type="submit" aria-label="Send message">
            <Send size={14} />
          </button>
        </form>
      </AuiProvider>
    </WorkspaceBlockFrame>
  );
}

export function RunTraceTimeline({ set }: ViewRenderProps) {
  return (
    <WorkspaceBlockFrame title="Run trace" eyebrow="Trace">
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
    <WorkspaceBlockFrame title="Tools" eyebrow="Activity">
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
    <WorkspaceBlockFrame title="Context" eyebrow="Atoms">
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

interface PMRow {
  id: string;
  title: string;
  status: string;
  priority: string;
}

const pmColumnHelper = createColumnHelper<PMRow>();
const pmColumns = [
  pmColumnHelper.accessor("title", {
    header: "Object",
    cell: (info) => info.getValue(),
  }),
  pmColumnHelper.accessor("status", {
    header: "Status",
    cell: (info) => <Badge tone={info.getValue() === "done" ? "live" : "neutral"}>{info.getValue()}</Badge>,
  }),
  pmColumnHelper.accessor("priority", {
    header: "Priority",
    cell: (info) => info.getValue(),
  }),
];

export function PMObjectPanel({ set }: ViewRenderProps) {
  const data = React.useMemo<PMRow[]>(
    () =>
      set.objects.map((object) => ({
        id: object.id,
        title: stringProp(object, "title") ?? object.id,
        status: stringProp(object, "status") ?? "open",
        priority: stringProp(object, "priority") ?? "normal",
      })),
    [set.objects],
  );
  const table = useReactTable({ data, columns: pmColumns, getCoreRowModel: getCoreRowModel() });

  return (
    <WorkspaceBlockFrame title="Records" eyebrow="Work">
      <table className="cpw-object-table">
        <thead>
          {table.getHeaderGroups().map((headerGroup) => (
            <tr key={headerGroup.id}>
              {headerGroup.headers.map((header) => (
                <th key={header.id}>
                  {header.isPlaceholder ? null : flexRender(header.column.columnDef.header, header.getContext())}
                </th>
              ))}
            </tr>
          ))}
        </thead>
        <tbody>
          {table.getRowModel().rows.map((row) => (
            <tr key={row.id}>
              {row.getVisibleCells().map((cell) => (
                <td key={cell.id}>{flexRender(cell.column.columnDef.cell, cell.getContext())}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </WorkspaceBlockFrame>
  );
}

export function SceneArtifactPreviewPanel({ set }: ViewRenderProps) {
  const artifact = set.objects[0];
  const response = sceneArtifactOpenUiResponse({
    title: stringProp(artifact, "title") ?? "Scene artifact",
    sceneId: stringProp(artifact, "scene_id") ?? artifact?.id ?? "scene:workspace",
    summary: stringProp(artifact, "summary"),
    atoms: stringProp(artifact, "atoms"),
  });

  return (
    <WorkspaceBlockFrame title="Scene" eyebrow="Artifact">
      <Renderer
        response={response}
        library={commonplaceOpenUiLibrary}
        isStreaming={false}
        onError={(errors) => {
          if (errors.length) {
            console.warn("OpenUI SceneArtifactPreview rejected response", errors);
          }
        }}
      />
    </WorkspaceBlockFrame>
  );
}

export function TerminalPanel({ set }: ViewRenderProps) {
  const command = stringProp(set.objects[0], "command") ?? "npm run lint";
  const output = stringProp(set.objects[0], "output") ?? "ready";
  const terminalRef = React.useRef<HTMLDivElement | null>(null);

  React.useEffect(() => {
    const element = terminalRef.current;
    if (!element) return;

    const terminal = new Terminal({
      convertEol: true,
      cursorBlink: false,
      disableStdin: true,
      fontFamily: "var(--cp-font-mono)",
      fontSize: 12,
      lineHeight: 1.45,
      rows: 7,
      theme: {
        background: "var(--cp-xterm-bg)",
        foreground: "var(--cp-xterm-fg)",
        cursor: "var(--cp-red)",
        selectionBackground: "var(--cp-xterm-selection)",
        black: "var(--cp-xterm-bg)",
        red: "var(--cp-red)",
        green: "var(--cp-green)",
        yellow: "var(--cp-gold)",
        blue: "var(--cp-teal)",
        magenta: "var(--cp-red)",
        cyan: "var(--cp-teal)",
        white: "var(--cp-xterm-fg)",
        brightBlack: "var(--cp-text-faint)",
        brightRed: "var(--cp-red)",
        brightGreen: "var(--cp-green)",
        brightYellow: "var(--cp-gold)",
        brightBlue: "var(--cp-teal)",
        brightMagenta: "var(--cp-red)",
        brightCyan: "var(--cp-teal)",
        brightWhite: "var(--cp-xterm-bright)",
      },
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(element);
    fitAddon.fit();
    terminal.write(`$ ${command}\r\n${output}`);

    const resizeObserver = new ResizeObserver(() => fitAddon.fit());
    resizeObserver.observe(element);

    return () => {
      resizeObserver.disconnect();
      terminal.dispose();
    };
  }, [command, output]);

  return (
    <WorkspaceBlockFrame title="Terminal" eyebrow="Shell">
      <div className="cpw-terminal">
        <div className="cpw-terminal-command">
          <SquareTerminal size={13} />
          <span>$ {command}</span>
        </div>
        <div ref={terminalRef} className="cpw-xterm-host" aria-label="Terminal output" />
      </div>
    </WorkspaceBlockFrame>
  );
}

export function AgentRunBoard({ set }: ViewRenderProps) {
  const columns = ["queued", "running", "blocked", "done"];

  return (
    <WorkspaceBlockFrame title="Agent runs" eyebrow="AgentRunBoard">
      <DndContext id="commonplace-agent-run-board">
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
                    <RunCard key={run.id} run={run} />
                  ))}
                </div>
              </div>
            );
          })}
        </div>
      </DndContext>
    </WorkspaceBlockFrame>
  );
}

export function CodeEditorPanel({ set, host }: ViewRenderProps) {
  const file = set.objects.find((object) => stringProp(object, "active") === "true") ?? set.objects[0];
  const initialContent = stringProp(file, "content") ?? "";
  const language = languageForPath(stringProp(file, "path"));

  return (
    <WorkspaceBlockFrame
      title={stringProp(file, "path") ?? "Editor"}
      eyebrow="Editor"
    >
      <CodeEditorSurface
        key={file?.id ?? "empty"}
        file={file}
        host={host}
        initialContent={initialContent}
        language={language}
      />
    </WorkspaceBlockFrame>
  );
}

function CodeEditorSurface({
  file,
  host,
  initialContent,
  language,
}: {
  file: ObjectRef | undefined;
  host: ViewRenderProps["host"];
  initialContent: string;
  language: CommonPlaceCodeLanguage;
}) {
  const [content, setContent] = React.useState(initialContent);

  return (
    <div className="cpw-codemirror-shell">
      <CodeMirror
        value={content}
        theme={commonplaceCodeMirrorTheme}
        extensions={commonplaceCodeExtensions(language)}
        basicSetup={{ lineNumbers: true, foldGutter: false, highlightActiveLine: true }}
        onChange={setContent}
        onBlur={() => {
          if (!file || content === initialContent) return;
          void host.emit({ kind: "update", id: file.id, patch: { content } });
        }}
        height="100%"
        minHeight="340px"
      />
    </div>
  );
}

interface FileTreeNode {
  readonly id: string;
  readonly name: string;
  readonly path: string;
  readonly kind: "directory" | "file";
  readonly active: boolean;
  readonly objectId?: string;
  readonly children?: FileTreeNode[];
}

function FileTreeNode({ node, style }: NodeRendererProps<FileTreeNode>) {
  const Icon = node.data.kind === "directory" ? FolderOpen : FileCode2;
  return (
    <div
      style={style}
      className="cpw-file-row"
      data-active={node.data.active ? "true" : "false"}
      data-kind={node.data.kind}
      onClick={() => {
        if (node.data.kind === "directory") {
          node.toggle();
        } else {
          node.activate();
        }
      }}
    >
      {node.data.kind === "directory" ? (
        <ChevronRight size={12} className="cpw-file-chevron" data-open={node.isOpen ? "true" : "false"} />
      ) : (
        <span className="cpw-file-chevron" />
      )}
      <Icon size={13} />
      <span className="truncate">{node.data.name}</span>
    </div>
  );
}

function RunCard({ run }: { run: ObjectRef }) {
  const { attributes, listeners, setNodeRef, transform, isDragging } = useDraggable({ id: run.id });
  const style: CSSProperties = transform
    ? { transform: `translate3d(${transform.x}px, ${transform.y}px, 0)` }
    : {};

  return (
    <article
      ref={setNodeRef}
      className="cpw-run-card"
      data-dragging={isDragging ? "true" : "false"}
      style={style}
      {...listeners}
      {...attributes}
    >
      <div className="cpw-run-title">
        <TimerReset size={13} />
        <span>{stringProp(run, "title")}</span>
      </div>
      <p>{stringProp(run, "summary")}</p>
    </article>
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

function toExternalThreadMessages(objects: readonly ObjectRef[]): ExternalThreadMessage[] {
  return objects.map((object) => {
    const role = stringProp(object, "role") === "user" ? "user" : "assistant";
    return {
      id: object.id,
      role,
      content: [{ type: "text", text: stringProp(object, "content") ?? "" }],
      createdAt: new Date(Number(stringProp(object, "created_at_ms")) || 1782420000000),
      metadata: { custom: {} },
      ...(role === "assistant"
        ? { status: { type: "complete", reason: "stop" } }
        : { attachments: [] }),
    } as ExternalThreadMessage;
  });
}

function buildFileTree(files: readonly ObjectRef[]): FileTreeNode[] {
  const roots = new Map<string, MutableFileTreeNode>();

  for (const file of files) {
    const path = stringProp(file, "path") ?? file.id;
    const parts = path.split("/").filter(Boolean);
    let current = roots;
    let prefix = "";

    parts.forEach((part, index) => {
      prefix = prefix ? `${prefix}/${part}` : part;
      const leaf = index === parts.length - 1;
      const existing = current.get(part);
      const next =
        existing ??
        {
          id: leaf ? file.id : `dir:${prefix}`,
          name: part,
          path: prefix,
          kind: leaf && stringProp(file, "content") ? "file" : "directory",
          active: stringProp(file, "active") === "true",
          objectId: leaf ? file.id : undefined,
          children: new Map<string, MutableFileTreeNode>(),
        };

      if (leaf) {
        next.id = file.id;
        next.kind = stringProp(file, "content") ? "file" : "directory";
        next.active = stringProp(file, "active") === "true";
        next.objectId = file.id;
      }

      current.set(part, next);
      current = next.children;
    });
  }

  return materializeNodes(roots);
}

interface MutableFileTreeNode {
  id: string;
  name: string;
  path: string;
  kind: "directory" | "file";
  active: boolean;
  objectId?: string;
  children: Map<string, MutableFileTreeNode>;
}

function materializeNodes(nodes: Map<string, MutableFileTreeNode>): FileTreeNode[] {
  return [...nodes.values()]
    .sort((a, b) => {
      if (a.kind !== b.kind) return a.kind === "directory" ? -1 : 1;
      return a.name.localeCompare(b.name);
    })
    .map((node) => ({
      id: node.id,
      name: node.name,
      path: node.path,
      kind: node.kind,
      active: node.active,
      objectId: node.objectId,
      children: node.children.size ? materializeNodes(node.children) : undefined,
    }));
}

function countTreeNodes(nodes: readonly FileTreeNode[]): number {
  return nodes.reduce((count, node) => count + 1 + countTreeNodes(node.children ?? []), 0);
}

function languageForPath(path: string | undefined): CommonPlaceCodeLanguage {
  if (!path) return "typescript";
  if (path.endsWith(".md") || path.endsWith(".mdx")) return "markdown";
  if (path.endsWith(".js") || path.endsWith(".jsx") || path.endsWith(".mjs")) return "javascript";
  if (path.endsWith(".rs")) return "rust";
  if (path.endsWith(".ts") || path.endsWith(".tsx")) return "typescript";
  return "text";
}

function fallbackDiff(side: string): string {
  return side === "before"
    ? "function renderPatch(patch) {\n  return patch.diff;\n}"
    : "function renderPatch(patch) {\n  return viewFor(patch.shape).render(patch);\n}";
}
