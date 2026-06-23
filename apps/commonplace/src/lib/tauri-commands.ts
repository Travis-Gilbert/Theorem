"use client";

import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./commonplace-client";

export type HarnessTarget = "local" | "hosted";

export interface HarnessSettings {
  endpoint: string;
  localEndpoint: string;
  activeTarget: HarnessTarget;
  tenant: string;
  bearerPresent: boolean;
}

export interface LocalNodeStatus {
  nodeUp: boolean;
  endpoint: string;
  port: number;
  storePath: string;
  activeTarget: HarnessTarget;
  toolsMatchHosted: boolean;
}

export interface ReceiverSettings {
  enabled: boolean;
  claimIntervalSecs: number;
  worktrees: Record<string, string>;
}

export interface ReceiverStatus {
  enabled: boolean;
  state: "off" | "configured" | "running" | "error";
  lanes: string[];
  lastClaimTime?: string;
  lastJobResult?: string;
}

export interface RoomContext {
  feed: Array<{ id: string; actor: string; text: string; createdAt?: string; kind?: string }>;
  participants: Array<{ actor: string; status: string; lastSeen?: string }>;
  intents: Array<{ actor: string; status: string; summary: string; footprint: string[] }>;
  records: Array<{ id: string; kind: string; title?: string; actor?: string }>;
}

export interface SyncReceipt {
  id: string;
  status: string;
  startedAt: string;
  finishedAt?: string;
  mergedNodes?: number;
  mergedEdges?: number;
  conflicts?: number;
  message: string;
}

export interface PageContext {
  url: string;
  title: string;
  text: string;
}

export interface AgentTabIngestInput {
  tabId: string;
  url: string;
  title?: string;
  text: string;
}

export async function localNodeStatus(): Promise<LocalNodeStatus> {
  if (isTauri()) return invoke<LocalNodeStatus>("local_node_status");
  return {
    nodeUp: false,
    endpoint: "http://127.0.0.1:17888/mcp",
    port: 17888,
    storePath: "~/Library/Application Support/Theorem/store",
    activeTarget: "local",
    toolsMatchHosted: false
  };
}

export async function harnessSettingsGet(): Promise<HarnessSettings | null> {
  if (isTauri()) return invoke<HarnessSettings>("harness_settings_get");
  return null;
}

export async function harnessSettingsSet(settings: HarnessSettings): Promise<void> {
  if (isTauri()) return invoke("harness_settings_set", { settings });
}

export async function keychainSet(provider: string, key: string): Promise<void> {
  if (isTauri()) return invoke("keychain_set", { provider, key });
}

export async function keychainHas(provider: string): Promise<boolean> {
  if (isTauri()) return invoke<boolean>("keychain_has", { provider });
  return false;
}

export async function keychainDelete(provider: string): Promise<void> {
  if (isTauri()) return invoke("keychain_delete", { provider });
}

export async function harnessBearerSet(token: string): Promise<void> {
  if (isTauri()) return invoke("harness_bearer_set", { token });
}

export async function harnessBearerClear(): Promise<void> {
  if (isTauri()) return invoke("harness_bearer_clear");
}

export async function receiverSettingsGet(): Promise<ReceiverSettings | null> {
  if (isTauri()) return invoke<ReceiverSettings>("receiver_settings_get");
  return null;
}

export async function receiverSettingsSet(settings: ReceiverSettings): Promise<void> {
  if (isTauri()) return invoke("receiver_settings_set", { settings });
}

export async function receiverStatus(): Promise<ReceiverStatus> {
  if (isTauri()) return invoke<ReceiverStatus>("receiver_status");
  return { enabled: false, state: "off", lanes: [] };
}

export async function spaceBindRoom(roomId: string, spaceName: string): Promise<void> {
  if (isTauri()) return invoke("space_bind_room", { input: { roomId, spaceName } });
}

export async function roomContext(roomId: string): Promise<RoomContext> {
  if (isTauri()) return invoke<RoomContext>("room_context", { roomId });
  return { feed: [], participants: [], intents: [], records: [] };
}

export async function roomPostMessage(roomId: string, message: string): Promise<void> {
  if (isTauri()) return invoke("room_post_message", { input: { roomId, message } });
}

export async function jobSubmit(input: {
  title: string;
  specRef: string;
  repo: string;
  kind: "ImplementSpec" | "Feature" | "Edit" | "App" | "Investigation";
  priority?: "P0" | "P1" | "P2";
  targetHead?: "ClaudeCode" | "Codex" | "Either";
}): Promise<void> {
  if (isTauri()) return invoke("job_submit", { input });
}

export async function queueStatus(input: { repo?: string; status?: string } = {}): Promise<unknown[]> {
  if (isTauri()) return invoke<unknown[]>("queue_status", { input });
  return [];
}

export async function syncRun(): Promise<SyncReceipt> {
  if (isTauri()) return invoke<SyncReceipt>("sync_run");
  return {
    id: "browser-preview",
    status: "disabled",
    startedAt: new Date().toISOString(),
    message: "Sync is only available inside the desktop shell."
  };
}

export async function tabCreate(tabId: string, url?: string): Promise<void> {
  if (isTauri()) return invoke("tab_create", { tabId, url: url ?? null });
}

export async function tabNavigate(tabId: string, url: string): Promise<void> {
  if (isTauri()) return invoke("tab_navigate", { tabId, url });
}

export async function extractVisibleText(tabId: string): Promise<PageContext> {
  if (isTauri()) return invoke<PageContext>("extract_visible_text", { tabId });
  return { url: "https://example.com", title: "Example", text: "Preview page text." };
}

export async function agentTabIngest(input: AgentTabIngestInput): Promise<unknown> {
  if (isTauri()) return invoke("agent_tab_ingest", { input });
  return { id: "browser-preview", status: "mocked" };
}
