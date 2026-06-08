// App state spine for Theorem Desktop. useReducer + context, no external state
// library. The persisted subset (SessionState: tabs, spaces, activeTabId) is
// saved to SQLite via the backend (D3) and restored on launch. Conversations,
// recall, and UI flags are in-memory.

import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  type ReactNode,
} from "react";
import * as cmd from "../lib/commands";
import { domainOf, routeOmnibox } from "../lib/routing";
import type {
  AppState,
  ChatTurn,
  MentionRef,
  ProviderId,
  Settings,
  SessionState,
  Space,
  SpaceId,
  SyncReceipt,
  Tab,
  TabId,
  TurnUsage,
} from "./types";

const HOSTED_ENDPOINT =
  "https://rustyredcore-theorem-production.up.railway.app/mcp";

const DEFAULT_SETTINGS: Settings = {
  harness: {
    endpoint: HOSTED_ENDPOINT,
    localEndpoint: "http://127.0.0.1:17888/mcp",
    activeTarget: "hosted",
    tenant: "default",
    bearerPresent: false,
  },
  receiver: {
    enabled: false,
    claimIntervalSecs: 20,
    worktrees: {
      "Travis-Gilbert/theorem":
        "/Users/travisgilbert/Tech Dev Local/Creative/Website/Theorem",
    },
  },
  sync: {
    enabled: false,
    intervalSecs: 300,
  },
  backgroundFetch: {
    enabled: false,
    intervalSecs: 900,
  },
  ollama: {
    endpoint: "http://127.0.0.1:11434",
    model: "llama3.2",
  },
  defaultModel: "deepseek",
  providerKeyPresent: { anthropic: false, openai: false, deepseek: false, ollama: false },
};

const initialState: AppState = {
  tabs: [],
  spaces: [],
  activeTabId: null,
  conversations: {},
  recallByDomain: {},
  roomFeedBySpace: {},
  participantsBySpace: {},
  queueJobs: [],
  syncReceipts: [],
  agentIngestionReceipts: [],
  costSummary: { turns: 0, tokensIn: 0, tokensOut: 0, estimatedUsd: 0 },
  railVisible: true,
  railView: "chat",
  queuePanelOpen: false,
  settingsOpen: false,
  settings: DEFAULT_SETTINGS,
};

type Action =
  | { type: "INIT_SESSION"; payload: SessionState }
  | { type: "ADD_TAB"; tab: Tab; activate?: boolean }
  | { type: "UPDATE_TAB"; tabId: TabId; patch: Partial<Tab> }
  | { type: "CLOSE_TAB"; tabId: TabId }
  | { type: "SET_ACTIVE_TAB"; tabId: TabId | null }
  | { type: "TOGGLE_PIN"; tabId: TabId }
  | { type: "REORDER_TABS"; order: TabId[] }
  | { type: "MOVE_TAB_TO_SPACE"; tabId: TabId; spaceId?: SpaceId }
  | { type: "ADD_SPACE"; space: Space }
  | { type: "UPDATE_SPACE"; spaceId: SpaceId; patch: Partial<Space> }
  | { type: "RENAME_SPACE"; spaceId: SpaceId; name: string }
  | { type: "DELETE_SPACE"; spaceId: SpaceId }
  | { type: "ADD_TURN"; tabId: TabId; turn: ChatTurn }
  | { type: "UPDATE_TURN"; tabId: TabId; turnId: string; patch: Partial<ChatTurn> }
  | { type: "SET_RAIL_VISIBLE"; visible: boolean }
  | { type: "SET_SETTINGS_OPEN"; open: boolean }
  | { type: "SET_SETTINGS"; patch: Partial<Settings> }
  | { type: "SET_RECEIVER_SETTINGS"; patch: Partial<Settings["receiver"]> }
  | { type: "SET_SYNC_SETTINGS"; patch: Partial<Settings["sync"]> }
  | { type: "SET_BACKGROUND_FETCH_SETTINGS"; patch: Partial<Settings["backgroundFetch"]> }
  | { type: "SET_OLLAMA_SETTINGS"; patch: Partial<Settings["ollama"]> }
  | { type: "SET_PROVIDER_KEY_PRESENT"; provider: ProviderId; present: boolean }
  | { type: "SET_RECALL"; domain: string; hits: AppState["recallByDomain"][string] }
  | { type: "SET_RAIL_VIEW"; view: AppState["railView"] }
  | { type: "SET_QUEUE_PANEL_OPEN"; open: boolean }
  | { type: "SET_ROOM_FEED"; spaceId: SpaceId; feed: AppState["roomFeedBySpace"][string] }
  | {
      type: "SET_PARTICIPANTS";
      spaceId: SpaceId;
      participants: AppState["participantsBySpace"][string];
    }
  | { type: "SET_QUEUE_JOBS"; jobs: AppState["queueJobs"] }
  | { type: "ADD_SYNC_RECEIPT"; receipt: SyncReceipt }
  | { type: "ADD_AGENT_INGESTION_RECEIPT"; receipt: AppState["agentIngestionReceipts"][number] }
  | { type: "ADD_USAGE"; usage: TurnUsage };

function reorderById(tabs: Tab[], order: TabId[]): Tab[] {
  const byId = new Map(tabs.map((t) => [t.id, t]));
  const next: Tab[] = [];
  for (const id of order) {
    const t = byId.get(id);
    if (t) {
      next.push(t);
      byId.delete(id);
    }
  }
  // Anything not named in the order keeps its relative position at the end.
  for (const t of tabs) if (byId.has(t.id)) next.push(t);
  return next;
}

function reducer(state: AppState, action: Action): AppState {
  switch (action.type) {
    case "INIT_SESSION":
      return {
        ...state,
        tabs: action.payload.tabs,
        spaces: action.payload.spaces,
        activeTabId: action.payload.activeTabId,
      };

    case "ADD_TAB": {
      const tabs = [...state.tabs, action.tab];
      return {
        ...state,
        tabs,
        activeTabId: action.activate ? action.tab.id : state.activeTabId,
      };
    }

    case "UPDATE_TAB":
      return {
        ...state,
        tabs: state.tabs.map((t) =>
          t.id === action.tabId ? { ...t, ...action.patch } : t,
        ),
      };

    case "CLOSE_TAB": {
      const idx = state.tabs.findIndex((t) => t.id === action.tabId);
      const tabs = state.tabs.filter((t) => t.id !== action.tabId);
      const conversations = { ...state.conversations };
      delete conversations[action.tabId];
      let activeTabId = state.activeTabId;
      if (state.activeTabId === action.tabId) {
        const fallback = tabs[idx] ?? tabs[idx - 1] ?? tabs[tabs.length - 1] ?? null;
        activeTabId = fallback ? fallback.id : null;
      }
      return { ...state, tabs, conversations, activeTabId };
    }

    case "SET_ACTIVE_TAB":
      return { ...state, activeTabId: action.tabId };

    case "TOGGLE_PIN":
      return {
        ...state,
        tabs: state.tabs.map((t) =>
          t.id === action.tabId ? { ...t, pinned: !t.pinned } : t,
        ),
      };

    case "REORDER_TABS":
      return { ...state, tabs: reorderById(state.tabs, action.order) };

    case "MOVE_TAB_TO_SPACE":
      return {
        ...state,
        tabs: state.tabs.map((t) =>
          t.id === action.tabId ? { ...t, spaceId: action.spaceId } : t,
        ),
      };

    case "ADD_SPACE":
      return { ...state, spaces: [...state.spaces, action.space] };

    case "UPDATE_SPACE":
      return {
        ...state,
        spaces: state.spaces.map((s) =>
          s.id === action.spaceId ? { ...s, ...action.patch } : s,
        ),
      };

    case "RENAME_SPACE":
      return {
        ...state,
        spaces: state.spaces.map((s) =>
          s.id === action.spaceId ? { ...s, name: action.name } : s,
        ),
      };

    case "DELETE_SPACE":
      return {
        ...state,
        spaces: state.spaces.filter((s) => s.id !== action.spaceId),
        // Tabs in the deleted space fall back to ungrouped.
        tabs: state.tabs.map((t) =>
          t.spaceId === action.spaceId ? { ...t, spaceId: undefined } : t,
        ),
      };

    case "ADD_TURN": {
      const prev = state.conversations[action.tabId]?.turns ?? [];
      return {
        ...state,
        conversations: {
          ...state.conversations,
          [action.tabId]: { tabId: action.tabId, turns: [...prev, action.turn] },
        },
      };
    }

    case "UPDATE_TURN": {
      const conv = state.conversations[action.tabId];
      if (!conv) return state;
      return {
        ...state,
        conversations: {
          ...state.conversations,
          [action.tabId]: {
            tabId: action.tabId,
            turns: conv.turns.map((t) =>
              t.id === action.turnId ? { ...t, ...action.patch } : t,
            ),
          },
        },
      };
    }

    case "SET_RAIL_VISIBLE":
      return { ...state, railVisible: action.visible };

    case "SET_SETTINGS_OPEN":
      return { ...state, settingsOpen: action.open };

    case "SET_SETTINGS":
      return { ...state, settings: { ...state.settings, ...action.patch } };

    case "SET_RECEIVER_SETTINGS":
      return {
        ...state,
        settings: {
          ...state.settings,
          receiver: { ...state.settings.receiver, ...action.patch },
        },
      };

    case "SET_SYNC_SETTINGS":
      return {
        ...state,
        settings: {
          ...state.settings,
          sync: { ...state.settings.sync, ...action.patch },
        },
      };

    case "SET_BACKGROUND_FETCH_SETTINGS":
      return {
        ...state,
        settings: {
          ...state.settings,
          backgroundFetch: { ...state.settings.backgroundFetch, ...action.patch },
        },
      };

    case "SET_OLLAMA_SETTINGS":
      return {
        ...state,
        settings: {
          ...state.settings,
          ollama: { ...state.settings.ollama, ...action.patch },
        },
      };

    case "SET_PROVIDER_KEY_PRESENT":
      return {
        ...state,
        settings: {
          ...state.settings,
          providerKeyPresent: {
            ...state.settings.providerKeyPresent,
            [action.provider]: action.present,
          },
        },
      };

    case "SET_RECALL":
      return {
        ...state,
        recallByDomain: { ...state.recallByDomain, [action.domain]: action.hits },
      };

    case "SET_RAIL_VIEW":
      return { ...state, railView: action.view };

    case "SET_QUEUE_PANEL_OPEN":
      return { ...state, queuePanelOpen: action.open };

    case "SET_ROOM_FEED":
      return {
        ...state,
        roomFeedBySpace: { ...state.roomFeedBySpace, [action.spaceId]: action.feed },
      };

    case "SET_PARTICIPANTS":
      return {
        ...state,
        participantsBySpace: {
          ...state.participantsBySpace,
          [action.spaceId]: action.participants,
        },
      };

    case "SET_QUEUE_JOBS":
      return { ...state, queueJobs: action.jobs };

    case "ADD_SYNC_RECEIPT":
      return { ...state, syncReceipts: [action.receipt, ...state.syncReceipts].slice(0, 20) };

    case "ADD_AGENT_INGESTION_RECEIPT":
      return {
        ...state,
        agentIngestionReceipts: [action.receipt, ...state.agentIngestionReceipts].slice(0, 20),
      };

    case "ADD_USAGE":
      return {
        ...state,
        costSummary: {
          turns: state.costSummary.turns + 1,
          tokensIn: state.costSummary.tokensIn + action.usage.tokensIn,
          tokensOut: state.costSummary.tokensOut + action.usage.tokensOut,
          estimatedUsd: state.costSummary.estimatedUsd + action.usage.estimatedUsd,
        },
      };

    default:
      return state;
  }
}

function newId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `id-${Date.now()}-${Math.floor(Math.random() * 1e9)}`;
}

function makeNewTab(): Tab {
  return { id: newId(), kind: "newtab", url: "", title: "New Tab", pinned: false };
}

/** Async action orchestrators: they own command side effects + dispatch. */
export interface Actions {
  newTab: () => void;
  newAgentTab: () => Promise<void>;
  /** Route omnibox input: navigate the active tab or open a rail chat turn. */
  submitOmnibox: (raw: string) => Promise<void>;
  navigateActive: (url: string) => Promise<void>;
  closeTab: (tabId: TabId) => Promise<void>;
  selectTab: (tabId: TabId) => Promise<void>;
  togglePin: (tabId: TabId) => void;
  reorderTabs: (order: TabId[]) => void;
  moveTabToSpace: (tabId: TabId, spaceId?: SpaceId) => void;
  addSpace: (name: string) => void;
  renameSpace: (spaceId: SpaceId, name: string) => void;
  deleteSpace: (spaceId: SpaceId) => void;
  toggleRail: () => void;
  openSettings: (open: boolean) => void;
  setHarness: (patch: Partial<Settings["harness"]>) => void;
  setReceiver: (patch: Partial<Settings["receiver"]>) => void;
  setSync: (patch: Partial<Settings["sync"]>) => void;
  setBackgroundFetch: (patch: Partial<Settings["backgroundFetch"]>) => void;
  setOllama: (patch: Partial<Settings["ollama"]>) => void;
  setDefaultModel: (model: ProviderId) => void;
  setProviderKeyPresent: (provider: ProviderId, present: boolean) => void;
  setRailView: (view: AppState["railView"]) => void;
  setQueuePanelOpen: (open: boolean) => void;
  bindSpaceToRoom: (spaceId: SpaceId) => Promise<void>;
  refreshRoom: (spaceId: SpaceId) => Promise<void>;
  postRoomMessage: (spaceId: SpaceId, message: string) => Promise<void>;
  refreshQueue: () => Promise<void>;
  runSync: () => Promise<void>;
  ingestAgentTab: (tabId: TabId) => Promise<void>;
  /** Send a chat turn in the active tab's conversation, with @-mentions. */
  sendChat: (text: string, mentions: MentionRef[]) => Promise<void>;
}

interface Ctx {
  state: AppState;
  actions: Actions;
}

const AppContext = createContext<Ctx | null>(null);

export function AppProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(reducer, initialState);

  // Keep a ref to current state for async orchestrators (avoids stale closures).
  const stateRef = useRef(state);
  stateRef.current = state;

  // Load persisted session (or seed a single ask-first tab) on mount.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const session = await cmd.sessionLoad();
      if (cancelled) return;
      if (session && session.tabs.length > 0) {
        dispatch({ type: "INIT_SESSION", payload: session });
        await cmd.tabSetActive(session.activeTabId);
      } else {
        const tab = makeNewTab();
        dispatch({ type: "ADD_TAB", tab, activate: true });
        await cmd.tabSetActive(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Persist the session subset (debounced) whenever it changes.
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      const snapshot: SessionState = {
        tabs: state.tabs,
        spaces: state.spaces,
        activeTabId: state.activeTabId,
      };
      void cmd.sessionSave(snapshot);
    }, 400);
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    };
  }, [state.tabs, state.spaces, state.activeTabId]);

  useEffect(() => {
    if (!state.settings.sync.enabled) return;
    const run = async () => {
      const receipt = await cmd.syncRun();
      dispatch({ type: "ADD_SYNC_RECEIPT", receipt });
    };
    void run();
    const timer = window.setInterval(run, state.settings.sync.intervalSecs * 1000);
    return () => window.clearInterval(timer);
  }, [state.settings.sync.enabled, state.settings.sync.intervalSecs]);

  useEffect(() => {
    if (!state.settings.backgroundFetch.enabled) return;
    const warm = async () => {
      const urls = stateRef.current.tabs
        .filter((tab) => tab.kind !== "newtab" && tab.url)
        .map((tab) => tab.url);
      for (const url of urls) {
        const domain = domainOf(url);
        if (!domain) continue;
        const hits = await cmd.harnessRecall({ domain, limit: 6 });
        dispatch({ type: "SET_RECALL", domain, hits });
      }
      await cmd.backgroundFetchReceipt({ urls });
    };
    const timer = window.setInterval(warm, state.settings.backgroundFetch.intervalSecs * 1000);
    return () => window.clearInterval(timer);
  }, [
    state.settings.backgroundFetch.enabled,
    state.settings.backgroundFetch.intervalSecs,
    state.tabs,
  ]);

  const actions = useMemo<Actions>(() => {
    const captureActiveContext = async () => {
      const s = stateRef.current;
      const active = s.tabs.find((t) => t.id === s.activeTabId);
      if (!active || active.kind === "newtab") return undefined;
      try {
        return await cmd.extractVisibleText(active.id);
      } catch {
        return { url: active.url, title: active.title };
      }
    };

    const refreshRecallFor = async (url: string) => {
      const domain = domainOf(url);
      if (!domain) return;
      const hits = await cmd.harnessRecall({ domain, limit: 6 });
      dispatch({ type: "SET_RECALL", domain, hits });
    };

    const navigateActive = async (url: string) => {
      const s = stateRef.current;
      const active = s.tabs.find((t) => t.id === s.activeTabId);
      if (!active) {
        // No active tab: make one and navigate it.
        const tab: Tab = { ...makeNewTab(), kind: "web", url, title: url };
        dispatch({ type: "ADD_TAB", tab, activate: true });
        await cmd.tabCreate(tab.id, url);
        await cmd.tabSetActive(tab.id);
        void refreshRecallFor(url);
        return;
      }
      if (active.kind === "newtab") {
        dispatch({ type: "UPDATE_TAB", tabId: active.id, patch: { kind: "web", url, title: url, loading: true } });
        await cmd.tabCreate(active.id, url);
        await cmd.tabSetActive(active.id);
      } else if (!active.url) {
        dispatch({ type: "UPDATE_TAB", tabId: active.id, patch: { url, title: url, loading: true } });
        await cmd.tabCreate(active.id, url);
        await cmd.tabSetActive(active.id);
      } else {
        dispatch({ type: "UPDATE_TAB", tabId: active.id, patch: { url, loading: true } });
        await cmd.tabNavigate(active.id, url);
      }
      void refreshRecallFor(url);
    };

    const sendChat = async (text: string, mentions: MentionRef[]) => {
      const s = stateRef.current;
      const tabId = s.activeTabId;
      if (!tabId || !text.trim()) return;

      const pageContext = await captureActiveContext();

      // Pull extracted text for each mentioned tab into the turn.
      const enrichedMentions: MentionRef[] = mentions;
      const mentionContexts = await Promise.all(
        mentions.map(async (m) => {
          try {
            return await cmd.extractVisibleText(m.tabId);
          } catch {
            return { url: m.url, title: m.title };
          }
        }),
      );

      const userTurn: ChatTurn = {
        id: newId(),
        role: "user",
        text,
        mentions: enrichedMentions,
        pageContext,
        createdAt: Date.now(),
      };
      dispatch({ type: "ADD_TURN", tabId, turn: userTurn });

      const pendingId = newId();
      dispatch({
        type: "ADD_TURN",
        tabId,
        turn: { id: pendingId, role: "assistant", text: "", createdAt: Date.now(), pending: true },
      });

      // Compose the model messages: page + mention context, then the question.
      const contextBlocks: string[] = [];
      if (pageContext?.text) {
        contextBlocks.push(`[active tab: ${pageContext.title} <${pageContext.url}>]\n${pageContext.text}`);
      }
      mentionContexts.forEach((c, i) => {
        if (c?.text) contextBlocks.push(`[@${mentions[i].title} <${c.url}>]\n${c.text}`);
      });
      const system = contextBlocks.length
        ? `You are answering about the user's open browser tabs. Context:\n\n${contextBlocks.join("\n\n")}`
        : "You are a helpful browsing assistant.";

      let answer = "";
      let usage: TurnUsage | undefined;
      try {
        const res = await cmd.modelChat({
          model: s.settings.defaultModel,
          ollamaEndpoint: s.settings.ollama.endpoint,
          ollamaModel: s.settings.ollama.model,
          messages: [
            { role: "system", content: system },
            { role: "user", content: text },
          ],
        });
        answer = res.content;
        usage = res.usage;
      } catch (e) {
        answer = `Could not reach the model client: ${String(e)}`;
      }
      dispatch({
        type: "UPDATE_TURN",
        tabId,
        turnId: pendingId,
        patch: { text: answer, pending: false, usage },
      });
      if (usage) dispatch({ type: "ADD_USAGE", usage });

      // Write the turn + provenance to harness memory (D4).
      void cmd.harnessRemember({
        text: `Q: ${text}\nA: ${answer}`,
        url: pageContext?.url,
        title: pageContext?.title,
        tags: ["desktop-rail"],
        provenance: { mentions: mentions.map((m) => m.url), usage },
      });
    };

    const submitJobCommand = async (text: string) => {
      const raw = text.replace(/^\/job\s+/i, "").trim();
      const [titlePart, specPart] = raw.split("|").map((part) => part.trim());
      if (!titlePart || !specPart) {
        const tabId = stateRef.current.activeTabId;
        if (tabId) {
          dispatch({
            type: "ADD_TURN",
            tabId,
            turn: {
              id: newId(),
              role: "assistant",
              text: "Use /job <title> | <spec path>",
              createdAt: Date.now(),
            },
          });
        }
        return;
      }
      await cmd.jobSubmit({
        title: titlePart,
        specRef: specPart,
        repo: "Travis-Gilbert/theorem",
        kind: specPart.includes("HANDOFF") ? "App" : "Feature",
        priority: "P1",
        targetHead: "Either",
      });
      const jobs = await cmd.queueStatus({ repo: "Travis-Gilbert/theorem" });
      dispatch({ type: "SET_QUEUE_JOBS", jobs });
      dispatch({ type: "SET_QUEUE_PANEL_OPEN", open: true });
    };

    const refreshRoom = async (spaceId: SpaceId) => {
      const space = stateRef.current.spaces.find((candidate) => candidate.id === spaceId);
      if (!space?.roomId) return;
      const context = await cmd.roomContext(space.roomId);
      dispatch({ type: "SET_ROOM_FEED", spaceId, feed: context.feed });
      dispatch({ type: "SET_PARTICIPANTS", spaceId, participants: context.participants });
    };

    return {
      newTab: () => {
        const tab = makeNewTab();
        dispatch({ type: "ADD_TAB", tab, activate: true });
        void cmd.tabSetActive(null);
      },
      newAgentTab: async () => {
        const tab: Tab = {
          ...makeNewTab(),
          kind: "agent",
          title: "Agent Tab",
          agentIngestionEnabled: true,
        };
        dispatch({ type: "ADD_TAB", tab, activate: true });
        await cmd.tabSetActive(null);
      },
      submitOmnibox: async (raw: string) => {
        const route = routeOmnibox(raw);
        if (route.kind === "navigate") {
          await navigateActive(route.url);
        } else if (route.text.trim().startsWith("/job ")) {
          await submitJobCommand(route.text);
        } else if (route.text.trim()) {
          await sendChat(route.text, []);
        }
      },
      navigateActive,
      closeTab: async (tabId: TabId) => {
        dispatch({ type: "CLOSE_TAB", tabId });
        await cmd.tabClose(tabId);
        const next = stateRef.current.activeTabId;
        await cmd.tabSetActive(
          next && stateRef.current.tabs.find((t) => t.id === next)?.kind !== "newtab"
            ? next
            : null,
        );
      },
      selectTab: async (tabId: TabId) => {
        dispatch({ type: "SET_ACTIVE_TAB", tabId });
        const t = stateRef.current.tabs.find((x) => x.id === tabId);
        await cmd.tabSetActive(t && t.kind !== "newtab" ? tabId : null);
        if (t && t.kind !== "newtab" && t.url) void refreshRecallFor(t.url);
        if (t?.spaceId) void refreshRoom(t.spaceId);
      },
      togglePin: (tabId: TabId) => dispatch({ type: "TOGGLE_PIN", tabId }),
      reorderTabs: (order: TabId[]) => dispatch({ type: "REORDER_TABS", order }),
      moveTabToSpace: (tabId: TabId, spaceId?: SpaceId) =>
        dispatch({ type: "MOVE_TAB_TO_SPACE", tabId, spaceId }),
      addSpace: (name: string) =>
        dispatch({
          type: "ADD_SPACE",
          space: { id: newId(), name, order: stateRef.current.spaces.length },
        }),
      renameSpace: (spaceId: SpaceId, name: string) =>
        dispatch({ type: "RENAME_SPACE", spaceId, name }),
      deleteSpace: (spaceId: SpaceId) => dispatch({ type: "DELETE_SPACE", spaceId }),
      toggleRail: () =>
        dispatch({ type: "SET_RAIL_VISIBLE", visible: !stateRef.current.railVisible }),
      openSettings: (open: boolean) => dispatch({ type: "SET_SETTINGS_OPEN", open }),
      setHarness: (patch) =>
        dispatch({
          type: "SET_SETTINGS",
          patch: { harness: { ...stateRef.current.settings.harness, ...patch } },
        }),
      setReceiver: (patch) =>
        dispatch({ type: "SET_RECEIVER_SETTINGS", patch }),
      setSync: (patch) => dispatch({ type: "SET_SYNC_SETTINGS", patch }),
      setBackgroundFetch: (patch) =>
        dispatch({ type: "SET_BACKGROUND_FETCH_SETTINGS", patch }),
      setOllama: (patch) => dispatch({ type: "SET_OLLAMA_SETTINGS", patch }),
      setDefaultModel: (model) =>
        dispatch({ type: "SET_SETTINGS", patch: { defaultModel: model } }),
      setProviderKeyPresent: (provider, present) =>
        dispatch({ type: "SET_PROVIDER_KEY_PRESENT", provider, present }),
      setRailView: (view) => dispatch({ type: "SET_RAIL_VIEW", view }),
      setQueuePanelOpen: (open) => dispatch({ type: "SET_QUEUE_PANEL_OPEN", open }),
      bindSpaceToRoom: async (spaceId) => {
        const space = stateRef.current.spaces.find((candidate) => candidate.id === spaceId);
        if (!space) return;
        const roomId = space.roomId || `desktop-${space.name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${spaceId.slice(0, 8)}`;
        await cmd.spaceBindRoom({ roomId, spaceName: space.name });
        dispatch({ type: "UPDATE_SPACE", spaceId, patch: { roomId } });
        await refreshRoom(spaceId);
      },
      refreshRoom,
      postRoomMessage: async (spaceId, message) => {
        const space = stateRef.current.spaces.find((candidate) => candidate.id === spaceId);
        if (!space?.roomId || !message.trim()) return;
        await cmd.roomPostMessage({ roomId: space.roomId, message });
        await refreshRoom(spaceId);
      },
      refreshQueue: async () => {
        const jobs = await cmd.queueStatus({ repo: "Travis-Gilbert/theorem" });
        dispatch({ type: "SET_QUEUE_JOBS", jobs });
      },
      runSync: async () => {
        const receipt = await cmd.syncRun();
        dispatch({ type: "ADD_SYNC_RECEIPT", receipt });
      },
      ingestAgentTab: async (tabId) => {
        const tab = stateRef.current.tabs.find((candidate) => candidate.id === tabId);
        if (!tab || tab.kind !== "agent") return;
        const context = await cmd.extractVisibleText(tabId);
        const receipt = await cmd.agentTabIngest({
          tabId,
          url: context.url || tab.url,
          title: context.title || tab.title,
          text: context.text || "",
        });
        dispatch({ type: "ADD_AGENT_INGESTION_RECEIPT", receipt });
      },
      sendChat,
    };
  }, []);

  const value = useMemo<Ctx>(() => ({ state, actions }), [state, actions]);
  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): Ctx {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used within AppProvider");
  return ctx;
}
