import type { DotMatrixState } from "@/components/assistant-ui/dot-matrix";

export type ApiModelState =
  | "offline"
  | "idle"
  | "connecting"
  | "queued"
  | "routing"
  | "thinking"
  | "searching"
  | "reading"
  | "editing"
  | "streaming"
  | "waiting_for_tool"
  | "waiting_for_approval"
  | "syncing"
  | "uploading"
  | "downloading"
  | "success"
  | "warning"
  | "error"
  | "paused"
  | "stopped";

export interface ApiModelDotContract {
  readonly apiState: ApiModelState;
  readonly dotState: DotMatrixState;
  readonly terminal: boolean;
  readonly description: string;
}

export const API_MODEL_DOT_MATRIX_CONTRACT: readonly ApiModelDotContract[] = [
  { apiState: "offline", dotState: "offline", terminal: true, description: "Model endpoint cannot be reached." },
  { apiState: "idle", dotState: "idle", terminal: true, description: "Model is available and not assigned to the turn." },
  { apiState: "connecting", dotState: "connecting", terminal: false, description: "Runtime is opening a model stream." },
  { apiState: "queued", dotState: "waiting", terminal: false, description: "Turn is queued behind another model or tool." },
  { apiState: "routing", dotState: "searching", terminal: false, description: "Router is selecting the model or tool lane." },
  { apiState: "thinking", dotState: "thinking", terminal: false, description: "Model is planning before emitting user-visible text." },
  { apiState: "searching", dotState: "searching", terminal: false, description: "Model is searching memory, code, or web context." },
  { apiState: "reading", dotState: "downloading", terminal: false, description: "Model is ingesting file or context payloads." },
  { apiState: "editing", dotState: "syncing", terminal: false, description: "Model is preparing a patch or structured edit." },
  { apiState: "streaming", dotState: "streaming", terminal: false, description: "Assistant response is actively streaming." },
  { apiState: "waiting_for_tool", dotState: "waiting", terminal: false, description: "Assistant is waiting for a tool result." },
  { apiState: "waiting_for_approval", dotState: "paused", terminal: false, description: "Assistant is blocked on user approval." },
  { apiState: "syncing", dotState: "syncing", terminal: false, description: "Result is syncing to the graph or workspace." },
  { apiState: "uploading", dotState: "uploading", terminal: false, description: "Input artifact is uploading." },
  { apiState: "downloading", dotState: "downloading", terminal: false, description: "Output artifact is downloading." },
  { apiState: "success", dotState: "success", terminal: true, description: "Model completed successfully." },
  { apiState: "warning", dotState: "warning", terminal: true, description: "Model completed with a warning." },
  { apiState: "error", dotState: "error", terminal: true, description: "Model or tool failed." },
  { apiState: "paused", dotState: "paused", terminal: false, description: "Run is paused." },
  { apiState: "stopped", dotState: "stopped", terminal: true, description: "Run was stopped by user or policy." },
] as const;

export interface CodeAgentModelStatus {
  readonly id: "router" | "planner" | "editor" | "reviewer";
  readonly label: string;
  readonly state: ApiModelState;
}

export interface CodeDiffArtifact {
  readonly id: string;
  readonly title: string;
  readonly path: string;
  readonly before: string;
  readonly after: string;
  readonly additions: number;
  readonly deletions: number;
}

export function dotStateForApiModelState(apiState: ApiModelState): DotMatrixState {
  return API_MODEL_DOT_MATRIX_CONTRACT.find((entry) => entry.apiState === apiState)?.dotState ?? "info";
}
