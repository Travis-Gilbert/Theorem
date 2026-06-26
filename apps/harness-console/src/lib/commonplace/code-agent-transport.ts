import { HARNESS_URL } from "@/lib/harness";
import type { ApiModelState, CodeDiffArtifact } from "./code-agent-contract";

export const CODE_AGENT_TRANSPORTS = [
  { id: "api", label: "API agent", description: "Run agent:theorem through the product API." },
  { id: "acp:claude", label: "Claude ACP", description: "Launch Claude through the CommonPlace ACP host." },
  { id: "acp:codex", label: "Codex ACP", description: "Launch Codex through the CommonPlace ACP host." },
] as const;

export type CodeAgentTransportId = (typeof CODE_AGENT_TRANSPORTS)[number]["id"];

export interface CodeAgentProgress {
  message?: string;
  states?: Partial<Record<"router" | "planner" | "editor" | "reviewer", ApiModelState>>;
}

export interface CodeAgentRunInput {
  prompt: string;
  transport: CodeAgentTransportId;
  cwd?: string;
  scope?: readonly string[];
  onProgress?: (progress: CodeAgentProgress) => void;
}

export interface CodeAgentRunResult {
  text: string;
  diffs: readonly CodeDiffArtifact[];
  transport: CodeAgentTransportId;
}

interface FrontendOutbound {
  type:
    | "session_started"
    | "session_update"
    | "file_write_review"
    | "command_approval"
    | "command_output"
    | "error";
  session_id?: string | null;
  agent_id?: string | null;
  update?: unknown;
  review?: unknown;
  approval?: unknown;
  output?: unknown;
  message?: string;
}

export async function runCodeAgentTurn(input: CodeAgentRunInput): Promise<CodeAgentRunResult> {
  if (input.transport === "api") {
    return runProductAgent(input);
  }
  return runAcpAgent(input);
}

function transportLabel(id: CodeAgentTransportId) {
  return CODE_AGENT_TRANSPORTS.find((transport) => transport.id === id)?.label ?? id;
}

async function runProductAgent(input: CodeAgentRunInput): Promise<CodeAgentRunResult> {
  input.onProgress?.({
    message: "Routing through /api/theorem/agent.",
    states: { router: "routing", planner: "queued", editor: "idle", reviewer: "idle" },
  });

  const response = await fetch("/api/theorem/agent", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      tenant: "Travis-Gilbert",
      binding_id: "agent:theorem",
      task: input.prompt,
      scope: input.scope ?? [],
    }),
  });
  const payload = (await response.json().catch(() => ({}))) as unknown;

  if (!response.ok) {
    throw new Error(agentPayloadText(payload) || `Theorem agent API returned ${response.status}.`);
  }

  input.onProgress?.({
    message: "Composed agent returned.",
    states: { router: "success", planner: "success", editor: "success", reviewer: "success" },
  });

  return {
    text: agentPayloadText(payload) || "Theorem API agent returned an empty response.",
    diffs: [],
    transport: input.transport,
  };
}

function runAcpAgent(input: CodeAgentRunInput): Promise<CodeAgentRunResult> {
  input.onProgress?.({
    message: `Opening ${transportLabel(input.transport)} session through ACP.`,
    states: { router: "routing", planner: "queued", editor: "idle", reviewer: "idle" },
  });

  return new Promise((resolve, reject) => {
    const socket = new WebSocket(commonplaceAcpWebsocketUrl());
    const agentId = input.transport.slice("acp:".length);
    const events: string[] = [];
    const diffs: CodeDiffArtifact[] = [];
    let sessionId: string | null = null;
    let prompted = false;
    let settled = false;
    let idleTimer: number | null = null;

    const hardTimer = window.setTimeout(() => {
      settle(resolve, socket, {
        text: acpText(agentId, events, "ACP session is still running; later events will arrive through the host stream."),
        diffs,
        transport: input.transport,
      });
    }, 12_000);

    function resetIdleTimer() {
      if (idleTimer) window.clearTimeout(idleTimer);
      idleTimer = window.setTimeout(() => {
        if (!prompted || settled) return;
        settle(resolve, socket, {
          text: acpText(agentId, events, "ACP prompt was sent and is awaiting the agent's next event."),
          diffs,
          transport: input.transport,
        });
      }, 2_200);
    }

    function fail(error: Error) {
      if (settled) return;
      settled = true;
      window.clearTimeout(hardTimer);
      if (idleTimer) window.clearTimeout(idleTimer);
      socket.close();
      reject(error);
    }

    socket.addEventListener("open", () => {
      socket.send(
        JSON.stringify({
          type: "start_session",
          agent_id: agentId,
          cwd: input.cwd ?? "",
        }),
      );
    });

    socket.addEventListener("message", (event) => {
      const outbound = parseAcpEvent(event.data);
      if (!outbound) return;

      if (outbound.type === "error") {
        const message = outbound.message ?? "ACP host returned an error.";
        if (!prompted && events.length === 0) {
          fail(new Error(message));
          return;
        }
        events.push(`error: ${message}`);
        resetIdleTimer();
        return;
      }

      if (outbound.type === "session_started" && outbound.session_id) {
        sessionId = outbound.session_id;
        input.onProgress?.({
          message: `${transportLabel(input.transport)} session started.`,
          states: { router: "success", planner: "thinking", editor: "queued", reviewer: "idle" },
        });
        socket.send(
          JSON.stringify({
            type: "prompt",
            session_id: sessionId,
            text: input.prompt,
          }),
        );
        prompted = true;
        resetIdleTimer();
        return;
      }

      if (outbound.type === "file_write_review") {
        diffs.push(diffFromAcpReview(outbound.review, diffs.length));
        input.onProgress?.({
          message: "ACP staged a file review.",
          states: { router: "success", planner: "streaming", editor: "editing", reviewer: "queued" },
        });
        resetIdleTimer();
        return;
      }

      const summary = summarizeAcpOutbound(outbound);
      if (summary) events.push(summary);
      if (outbound.type === "command_approval") {
        input.onProgress?.({
          message: "ACP staged a command approval.",
          states: { router: "success", planner: "streaming", editor: "editing", reviewer: "queued" },
        });
      }
      resetIdleTimer();
    });

    socket.addEventListener("error", () => fail(new Error("Could not connect to the CommonPlace ACP websocket.")));

    socket.addEventListener("close", () => {
      if (settled) return;
      if (!sessionId) {
        fail(new Error("CommonPlace ACP websocket closed before session start."));
        return;
      }
      settle(resolve, socket, {
        text: acpText(agentId, events, "ACP session closed."),
        diffs,
        transport: input.transport,
      });
    });

    function settle(
      done: (result: CodeAgentRunResult) => void,
      ws: WebSocket,
      result: CodeAgentRunResult,
    ) {
      if (settled) return;
      settled = true;
      window.clearTimeout(hardTimer);
      if (idleTimer) window.clearTimeout(idleTimer);
      ws.close();
      input.onProgress?.({
        message: "ACP handoff completed.",
        states: { router: "success", planner: "success", editor: diffs.length ? "success" : "idle", reviewer: "success" },
      });
      done(result);
    }
  });
}

function commonplaceAcpWebsocketUrl() {
  const configured = process.env.NEXT_PUBLIC_COMMONPLACE_ACP_WS_URL;
  if (configured) return configured;
  const url = new URL(HARNESS_URL);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = "/v1/commonplace/acp/ws";
  url.search = "";
  return url.toString();
}

function parseAcpEvent(data: unknown): FrontendOutbound | null {
  if (typeof data !== "string") return null;
  try {
    return JSON.parse(data) as FrontendOutbound;
  } catch {
    return null;
  }
}

function summarizeAcpOutbound(outbound: FrontendOutbound) {
  if (outbound.type === "session_update") return `update: ${compactJson(outbound.update)}`;
  if (outbound.type === "command_approval") return `command approval: ${compactJson(outbound.approval)}`;
  if (outbound.type === "command_output") return `command output: ${compactJson(outbound.output)}`;
  return "";
}

function diffFromAcpReview(review: unknown, index: number): CodeDiffArtifact {
  const value = isRecord(review) ? review : {};
  const path = typeof value.path === "string" ? value.path : "workspace file";
  const before = typeof value.previous_content === "string" ? value.previous_content : "";
  const after = typeof value.content === "string" ? value.content : "";
  return {
    id: typeof value.request_id === "string" ? value.request_id : `acp-diff:${Date.now()}:${index}`,
    title: "ACP file write review",
    path,
    before,
    after,
    additions: Math.max(0, after.split("\n").length - before.split("\n").length),
    deletions: Math.max(0, before.split("\n").length - after.split("\n").length),
  };
}

function acpText(agentId: string, events: readonly string[], fallback: string) {
  const body = events.length ? events.slice(-5).join("\n") : fallback;
  return `${agentId} ACP\n${body}`;
}

function agentPayloadText(payload: unknown): string {
  if (typeof payload === "string") return payload;
  if (!isRecord(payload)) return "";
  if (typeof payload.message === "string") return payload.message;
  if (typeof payload.content === "string") return payload.content;
  if (typeof payload.text === "string") return payload.text;
  if ("error" in payload) return [payload.error, payload.message].filter((part) => typeof part === "string").join(": ");
  if ("result" in payload) return agentPayloadText(payload.result) || compactJson(payload.result);
  return compactJson(payload);
}

function compactJson(value: unknown) {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
