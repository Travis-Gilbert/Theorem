"use client";

import {
  AgentRunBoard,
  AgentThreadPanel,
  CodeEditorPanel,
  ContextArtifactDrawer,
  FileTreePanel,
  PMObjectPanel,
  PatchReviewPanel,
  RunTraceTimeline,
  SceneArtifactPreviewPanel,
  TerminalPanel,
  ToolActivityPanel,
} from "@/components/block-view/harness-panels";
import type { ViewDescriptor } from "./types";

export const HARNESS_VIEW_DESCRIPTORS: readonly ViewDescriptor[] = [
  {
    id: "file-tree",
    name: "FileTreePanel",
    accepts: {
      required_types: ["file"],
      required_edge: { edge: "CONTAINS", dir: "out" },
    },
    emits: ["open", "select"],
    source: {
      package: "react-arborist",
      component: "Tree",
      mode: "reskin",
      regime: "css-vars",
    },
    render: FileTreePanel,
  },
  {
    id: "code-editor",
    name: "CodeMirrorPanel",
    accepts: {
      required_types: ["file"],
      required_fields: ["content"],
      cardinality: "many",
    },
    emits: ["open", "update"],
    source: {
      package: "@uiw/react-codemirror",
      component: "CodeMirror",
      mode: "reskin",
      regime: "css-vars",
    },
    render: CodeEditorPanel,
  },
  {
    id: "patch-review",
    name: "PatchReviewPanel",
    accepts: {
      required_types: ["patch"],
      cardinality: "one",
    },
    emits: ["dispatch", "run_agent", "open"],
    source: {
      package: "react-codemirror-merge",
      component: "CodeMirrorMerge",
      mode: "wrap",
      regime: "css-vars",
    },
    render: PatchReviewPanel,
  },
  {
    id: "agent-thread",
    name: "AgentThreadPanel",
    accepts: {
      required_types: ["agent_message"],
      required_fields: ["content"],
      cardinality: "many",
    },
    emits: ["run_agent", "open"],
    source: {
      package: "@assistant-ui/react",
      component: "ExternalThread adapter",
      mode: "wrap",
      regime: "css-vars",
    },
    render: AgentThreadPanel,
  },
  {
    id: "run-trace",
    name: "RunTraceTimeline",
    accepts: {
      required_types: ["run_step"],
      required_fields: ["summary"],
      required_axes: { temporal: true },
      cardinality: "many",
    },
    emits: ["open"],
    source: {
      package: "@/components/ui",
      component: "shadcn timeline primitives",
      mode: "bespoke",
      regime: "css-vars",
      allowedBespokeReason: "Run lifecycle and validation handoffs are Theorem-specific semantics.",
    },
    render: RunTraceTimeline,
  },
  {
    id: "tool-activity",
    name: "ToolActivityPanel",
    accepts: {
      required_types: ["tool_activity"],
      required_fields: ["status"],
      cardinality: "many",
    },
    emits: ["invoke_tool", "open"],
    source: {
      package: "@assistant-ui/react",
      component: "Tool call adapter",
      mode: "wrap",
      regime: "css-vars",
    },
    render: ToolActivityPanel,
  },
  {
    id: "context-artifact",
    name: "ContextArtifactDrawer",
    accepts: {
      required_types: ["context_atom"],
      required_axes: { embeddable: true },
      cardinality: "many",
    },
    emits: ["open", "select"],
    source: {
      package: "@/components/ui",
      component: "shadcn Sheet/Table primitives",
      mode: "bespoke",
      regime: "css-vars",
      allowedBespokeReason: "Included atoms, token ledger, and provenance are Theseus context semantics.",
    },
    render: ContextArtifactDrawer,
  },
  {
    id: "terminal",
    name: "TerminalPanel",
    accepts: {
      required_types: ["terminal_session"],
      required_fields: ["command"],
      cardinality: "one",
    },
    emits: ["dispatch", "open"],
    source: {
      package: "@xterm/xterm",
      component: "Terminal",
      mode: "vendor",
      regime: "css-vars",
    },
    render: TerminalPanel,
  },
  {
    id: "pm-object",
    name: "PMObjectPanel",
    accepts: {
      required_types: ["pm_record"],
      required_fields: ["status"],
      cardinality: "many",
    },
    emits: ["open", "select", "update"],
    source: {
      package: "@tanstack/react-table",
      component: "useReactTable",
      mode: "wrap",
      regime: "css-vars",
    },
    render: PMObjectPanel,
  },
  {
    id: "scene-artifact-preview",
    name: "SceneArtifactPreview",
    accepts: {
      required_types: ["scene_artifact"],
      required_fields: ["scene_id"],
      cardinality: "one",
    },
    emits: ["open"],
    source: {
      package: "@openuidev/react-lang",
      component: "Renderer with registered SceneArtifactPreview component",
      mode: "wrap",
      regime: "scene",
    },
    render: SceneArtifactPreviewPanel,
  },
  {
    id: "agent-run-board",
    name: "AgentRunBoard",
    accepts: {
      required_types: ["agent_run"],
      required_fields: ["status"],
      required_axes: { temporal: true },
      cardinality: "many",
    },
    emits: ["run_agent", "dispatch", "open"],
    source: {
      package: "@dnd-kit/core",
      component: "DndContext plus shadcn Card/Badge primitives",
      mode: "bespoke",
      regime: "css-vars",
      allowedBespokeReason: "Agent run status, dependency, and worktree lifecycle are Theorem-specific semantics.",
    },
    render: AgentRunBoard,
  },
] as const;
