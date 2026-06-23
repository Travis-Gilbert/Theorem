"use client";

import {
  ArrowUpRight,
  Cloud,
  Database,
  Globe2,
  KeyRound,
  Loader2,
  MessageSquare,
  Plus,
  Radio,
  RefreshCw,
  Send,
  Sparkles,
  Waypoints,
  Zap
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  type CommonPlaceItem,
  createNote,
  engineConfig,
  fetchItems,
  subscribeToItemChanges
} from "@/lib/commonplace-client";
import {
  agentTabIngest,
  extractVisibleText,
  harnessSettingsGet,
  keychainSet,
  localNodeStatus,
  receiverSettingsGet,
  receiverSettingsSet,
  receiverStatus,
  roomContext,
  roomPostMessage,
  spaceBindRoom,
  syncRun,
  tabCreate,
  tabNavigate,
  type HarnessSettings,
  type LocalNodeStatus,
  type ReceiverSettings,
  type ReceiverStatus,
  type RoomContext,
  type SyncReceipt
} from "@/lib/tauri-commands";

const ROOM_ID = "commonplace-desktop";

function formatTime(ms: number): string {
  if (!ms) return "now";
  return new Intl.DateTimeFormat("en", {
    hour: "numeric",
    minute: "2-digit",
    month: "short",
    day: "numeric"
  }).format(new Date(ms));
}

function itemTone(kind: string): string {
  if (kind === "source") return "var(--cp-teal)";
  if (kind === "task") return "var(--cp-russet)";
  if (kind === "quote") return "var(--cp-gold)";
  if (kind === "concept") return "var(--cp-violet)";
  return "var(--cp-paper-ink)";
}

export function CommonPlaceDesktop(): React.ReactElement {
  const config = useMemo(engineConfig, []);
  const [items, setItems] = useState<CommonPlaceItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [node, setNode] = useState<LocalNodeStatus | null>(null);
  const [harness, setHarness] = useState<HarnessSettings | null>(null);
  const [receiver, setReceiver] = useState<ReceiverStatus | null>(null);
  const [receiverSettings, setReceiverSettings] = useState<ReceiverSettings | null>(null);
  const [room, setRoom] = useState<RoomContext>({ feed: [], participants: [], intents: [], records: [] });
  const [roomText, setRoomText] = useState("");
  const [noteTitle, setNoteTitle] = useState("");
  const [providerKey, setProviderKey] = useState("");
  const [browserUrl, setBrowserUrl] = useState("https://example.com");
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [syncReceipt, setSyncReceipt] = useState<SyncReceipt | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const selected = items.find((item) => item.id === selectedId) ?? items[0] ?? null;

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const [nextItems, nextNode, nextHarness, nextReceiver, nextReceiverSettings, nextRoom] =
        await Promise.all([
          fetchItems(),
          localNodeStatus(),
          harnessSettingsGet(),
          receiverStatus(),
          receiverSettingsGet(),
          roomContext(ROOM_ID)
        ]);
      setItems(nextItems);
      setNode(nextNode);
      setHarness(nextHarness);
      setReceiver(nextReceiver);
      setReceiverSettings(nextReceiverSettings);
      setRoom(nextRoom);
      setSelectedId((current) => current ?? nextItems[0]?.id ?? null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    }
  }, []);

  useEffect(() => {
    void refresh();
    return subscribeToItemChanges(() => {
      void refresh();
    });
  }, [refresh]);

  async function handleCreateNote(): Promise<void> {
    const title = noteTitle.trim();
    if (!title) return;
    setBusy(true);
    try {
      const item = await createNote(title);
      setNoteTitle("");
      setSelectedId(item.id);
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  async function handleStoreKey(): Promise<void> {
    const key = providerKey.trim();
    if (!key) return;
    setBusy(true);
    try {
      await keychainSet("openai", key);
      setProviderKey("");
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  async function handleReceiverToggle(): Promise<void> {
    const current = receiverSettings ?? {
      enabled: false,
      claimIntervalSecs: 20,
      worktrees: {}
    };
    setBusy(true);
    try {
      await receiverSettingsSet({ ...current, enabled: !current.enabled });
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  async function handleRoomPost(): Promise<void> {
    const message = roomText.trim();
    if (!message) return;
    setBusy(true);
    try {
      await spaceBindRoom(ROOM_ID, "CommonPlace Desktop");
      await roomPostMessage(ROOM_ID, message);
      setRoomText("");
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  async function handleBrowserOpen(): Promise<void> {
    const tabId = activeTabId ?? crypto.randomUUID();
    setActiveTabId(tabId);
    await tabCreate(tabId, browserUrl);
    await tabNavigate(tabId, browserUrl);
  }

  async function handleBrowserIngest(): Promise<void> {
    if (!activeTabId) return;
    setBusy(true);
    try {
      const page = await extractVisibleText(activeTabId);
      await agentTabIngest({
        tabId: activeTabId,
        url: page.url,
        title: page.title,
        text: page.text
      });
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  async function handleSync(): Promise<void> {
    setBusy(true);
    try {
      setSyncReceipt(await syncRun());
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="cp-shell">
      <aside className="cp-sidebar">
        <div className="cp-brand">
          <div className="cp-brand-mark">
            <Sparkles size={18} />
          </div>
          <div>
            <strong>CommonPlace</strong>
            <span>Desktop</span>
          </div>
        </div>
        <nav className="cp-nav" aria-label="CommonPlace sections">
          {["Home", "Library", "Models", "Artifacts", "Notebooks", "Projects", "Timeline", "Map", "Engine"].map(
            (label) => (
              <button key={label} type="button" className={label === "Library" ? "active" : ""}>
                {label}
              </button>
            )
          )}
        </nav>
        <div className="cp-status">
          <span className={node?.nodeUp ? "dot on" : "dot"} />
          <span>{node?.activeTarget ?? "local"}</span>
          <small>{config.tenant}</small>
        </div>
      </aside>

      <section className="cp-main">
        <header className="cp-topbar">
          <div className="cp-search">
            <Waypoints size={18} />
            <input
              value={noteTitle}
              onChange={(event) => setNoteTitle(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void handleCreateNote();
              }}
              placeholder="Capture a note, source, hunch, or task"
            />
            <button type="button" title="Create item" onClick={() => void handleCreateNote()} disabled={busy}>
              {busy ? <Loader2 size={16} className="spin" /> : <Plus size={16} />}
            </button>
          </div>
          <div className="cp-top-actions">
            <button type="button" title="Refresh" onClick={() => void refresh()}>
              <RefreshCw size={16} />
            </button>
            <button type="button" title="Sync" onClick={() => void handleSync()}>
              <Cloud size={16} />
            </button>
          </div>
        </header>

        {error ? <div className="cp-error">{error}</div> : null}

        <div className="cp-grid">
          <section className="cp-object-list" aria-label="Objects">
            <div className="cp-section-title">
              <Database size={16} />
              <span>Objects</span>
              <small>{items.length}</small>
            </div>
            <div className="cp-list">
              {items.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  className={item.id === selected?.id ? "cp-object active" : "cp-object"}
                  onClick={() => setSelectedId(item.id)}
                >
                  <span className="cp-kind" style={{ background: itemTone(item.kind) }} />
                  <span>
                    <strong>{item.title || item.id}</strong>
                    <small>{item.kind} / {item.source}</small>
                  </span>
                  <time>{formatTime(item.updatedAtMs)}</time>
                </button>
              ))}
            </div>
          </section>

          <section className="cp-detail" aria-label="Object detail">
            {selected ? (
              <>
                <div className="cp-detail-head">
                  <span className="cp-kind large" style={{ background: itemTone(selected.kind) }} />
                  <div>
                    <p>{selected.kind}</p>
                    <h1>{selected.title || selected.id}</h1>
                  </div>
                </div>
                <div className="cp-paper">
                  <p>Source: {selected.source}</p>
                  <p>Updated: {formatTime(selected.updatedAtMs)}</p>
                  <pre>{JSON.stringify(selected.extra ?? {}, null, 2)}</pre>
                </div>
              </>
            ) : (
              <div className="cp-empty">No objects yet</div>
            )}
          </section>

          <aside className="cp-panels" aria-label="Agent panels">
            <section className="cp-panel">
              <div className="cp-section-title">
                <Radio size={16} />
                <span>Local Node</span>
              </div>
              <dl>
                <div><dt>Endpoint</dt><dd>{node?.endpoint ?? config.graphqlUrl}</dd></div>
                <div><dt>Store</dt><dd>{node?.storePath ?? "browser preview"}</dd></div>
                <div><dt>Bearer</dt><dd>{harness?.bearerPresent ? "stored" : "empty"}</dd></div>
              </dl>
              <div className="cp-row">
                <input
                  value={providerKey}
                  onChange={(event) => setProviderKey(event.target.value)}
                  placeholder="Provider key"
                  type="password"
                />
                <button type="button" title="Store key" onClick={() => void handleStoreKey()}>
                  <KeyRound size={16} />
                </button>
              </div>
            </section>

            <section className="cp-panel">
              <div className="cp-section-title">
                <Zap size={16} />
                <span>Receiver</span>
                <small>{receiver?.state ?? "off"}</small>
              </div>
              <button type="button" className="cp-wide-action" onClick={() => void handleReceiverToggle()}>
                <Radio size={16} />
                <span>{receiverSettings?.enabled ? "Running" : "Stopped"}</span>
              </button>
              <p className="cp-muted">{receiver?.lanes.join(", ") || "No active lanes"}</p>
            </section>

            <section className="cp-panel">
              <div className="cp-section-title">
                <MessageSquare size={16} />
                <span>Coordination</span>
                <small>{room.feed.length}</small>
              </div>
              <div className="cp-feed">
                {room.feed.slice(-4).map((item) => (
                  <p key={item.id}><strong>{item.actor}</strong> {item.text}</p>
                ))}
              </div>
              <div className="cp-row">
                <input value={roomText} onChange={(event) => setRoomText(event.target.value)} placeholder="Room message" />
                <button type="button" title="Post" onClick={() => void handleRoomPost()}>
                  <Send size={16} />
                </button>
              </div>
            </section>

            <section className="cp-panel">
              <div className="cp-section-title">
                <Globe2 size={16} />
                <span>Co-browser</span>
              </div>
              <div className="cp-row">
                <input value={browserUrl} onChange={(event) => setBrowserUrl(event.target.value)} />
                <button type="button" title="Open" onClick={() => void handleBrowserOpen()}>
                  <ArrowUpRight size={16} />
                </button>
              </div>
              <button type="button" className="cp-wide-action" onClick={() => void handleBrowserIngest()} disabled={!activeTabId}>
                <Globe2 size={16} />
                <span>Ingest active page</span>
              </button>
            </section>

            {syncReceipt ? (
              <section className="cp-panel">
                <div className="cp-section-title">
                  <Cloud size={16} />
                  <span>Sync</span>
                  <small>{syncReceipt.status}</small>
                </div>
                <p className="cp-muted">{syncReceipt.message}</p>
              </section>
            ) : null}
          </aside>
        </div>
      </section>
    </main>
  );
}
