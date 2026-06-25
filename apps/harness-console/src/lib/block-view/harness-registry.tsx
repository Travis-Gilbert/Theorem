"use client";

import {
  AgentRunBoard,
  AgentThreadPanel,
  CodeEditorPanel,
  ContextArtifactDrawer,
  FileTreePanel,
  PatchReviewPanel,
  RunTraceTimeline,
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
    render: TerminalPanel,
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
    render: AgentRunBoard,
  },
] as const;
