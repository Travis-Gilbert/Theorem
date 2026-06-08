import { useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../state/store";
import type { SpaceId } from "../state/types";
import { TabItem } from "./TabItem";
import { GearIcon, PanelIcon, PlusIcon } from "./icons";

export function Sidebar() {
  const { state, actions } = useApp();
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const tabEls = useRef<Map<string, HTMLDivElement>>(new Map());

  const pinned = state.tabs.filter((t) => t.pinned);
  const unpinned = state.tabs.filter((t) => !t.pinned);
  const ungrouped = unpinned.filter((t) => !t.spaceId);
  const spaces = [...state.spaces].sort((a, b) => a.order - b.order);

  // Flat keyboard order across pinned, ungrouped, then each space (D3.3).
  const flatOrder = useMemo<string[]>(() => {
    const order = [...pinned.map((t) => t.id), ...ungrouped.map((t) => t.id)];
    for (const s of spaces) {
      for (const t of unpinned) if (t.spaceId === s.id) order.push(t.id);
    }
    return order;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.tabs, state.spaces]);

  const registerRef = (id: string, el: HTMLDivElement | null) => {
    if (el) tabEls.current.set(id, el);
    else tabEls.current.delete(id);
  };
  const focusTab = (id: string) => tabEls.current.get(id)?.focus();

  // Cmd/Ctrl+1..9 jumps to the Nth tab in the visible order.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && /^[1-9]$/.test(e.key)) {
        const target = flatOrder[Number(e.key) - 1];
        if (target) {
          e.preventDefault();
          void actions.selectTab(target);
          focusTab(target);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [flatOrder, actions]);

  // Roving-tabindex keyboard contract for the tab list.
  const onKeyNav = (tabId: string, e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      // Keyboard alternative to drag reorder: nudge the tab in the global order.
      e.preventDefault();
      const ids = state.tabs.map((t) => t.id);
      const from = ids.indexOf(tabId);
      const to = from + (e.key === "ArrowDown" ? 1 : -1);
      if (from >= 0 && to >= 0 && to < ids.length) {
        ids.splice(to, 0, ids.splice(from, 1)[0]);
        actions.reorderTabs(ids);
        setTimeout(() => focusTab(tabId), 0);
      }
      return;
    }
    const i = flatOrder.indexOf(tabId);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      const next = flatOrder[Math.min(i + 1, flatOrder.length - 1)];
      if (next) {
        void actions.selectTab(next);
        focusTab(next);
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const prev = flatOrder[Math.max(i - 1, 0)];
      if (prev) {
        void actions.selectTab(prev);
        focusTab(prev);
      }
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      void actions.selectTab(tabId);
    } else if (e.key === "Delete" || ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "w")) {
      e.preventDefault();
      void actions.closeTab(tabId);
    }
  };

  // Per-tab ingestion count: receipts captured from this tab's URL (D4.2).
  const badgeFor = (url: string) =>
    state.agentIngestionReceipts.filter((r) => r.url === url && r.status === "ok").length;

  const onDropOnTab = (draggedId: string, targetId: string) => {
    const target = state.tabs.find((t) => t.id === targetId);
    if (target) actions.moveTabToSpace(draggedId, target.spaceId);
    const ids = state.tabs.map((t) => t.id).filter((id) => id !== draggedId);
    const at = ids.indexOf(targetId);
    ids.splice(at < 0 ? ids.length : at, 0, draggedId);
    actions.reorderTabs(ids);
    setDropTarget(null);
  };

  const onDropOnSpace = (e: React.DragEvent, spaceId?: SpaceId) => {
    e.preventDefault();
    const id = e.dataTransfer.getData("text/tab-id");
    if (id) actions.moveTabToSpace(id, spaceId);
    setDropTarget(null);
  };

  const renderList = (tabs: typeof state.tabs) =>
    tabs.map((t) => (
      <TabItem
        key={t.id}
        tab={t}
        active={t.id === state.activeTabId}
        badge={t.kind === "agent" ? badgeFor(t.url) : undefined}
        onSelect={actions.selectTab}
        onClose={actions.closeTab}
        onTogglePin={actions.togglePin}
        onDropOnTab={onDropOnTab}
        onKeyNav={onKeyNav}
        registerRef={registerRef}
      />
    ));

  return (
    <aside className="sidebar">
      <div className="sidebar__head">
        <span className="brand">
          <span className="brand__mark" />
          Theorem
        </span>
        <span className="sidebar__spacer" />
        <button className="iconbtn" onClick={actions.newTab} title="New tab (ask-first)">
          <PlusIcon />
        </button>
        <button className="iconbtn" onClick={() => void actions.newAgentTab()} title="New agent tab">
          A
        </button>
        <button
          className="iconbtn"
          onClick={() => actions.openSettings(true)}
          title="Settings"
        >
          <GearIcon />
        </button>
      </div>

      <div className="sidebar__body">
        {pinned.length > 0 && (
          <div>
            <div className="section__label">Pinned</div>
            <div className="pinned__grid">
              {pinned.map((t) => (
                <button
                  key={t.id}
                  className={
                    "pinned__item" +
                    (t.id === state.activeTabId ? " pinned__item--active" : "")
                  }
                  onClick={() => actions.selectTab(t.id)}
                  title={t.title || t.url}
                >
                  {t.favicon ? (
                    <img src={t.favicon} alt="" width={16} height={16} />
                  ) : (
                    (t.title || t.url || "T").slice(0, 1).toUpperCase()
                  )}
                </button>
              ))}
            </div>
          </div>
        )}

        <div>
          <div className="section__label">Tabs</div>
          <div
            className={"tablist" + (dropTarget === "ungrouped" ? " drop-target" : "")}
            role="tablist"
            aria-label="Tabs"
            aria-orientation="vertical"
            onDragOver={(e) => {
              e.preventDefault();
              setDropTarget("ungrouped");
            }}
            onDragLeave={() => setDropTarget(null)}
            onDrop={(e) => onDropOnSpace(e, undefined)}
          >
            {ungrouped.length === 0 ? (
              <div className="newtab__hint" style={{ padding: "4px 8px" }}>
                No open tabs.
              </div>
            ) : (
              renderList(ungrouped)
            )}
          </div>
        </div>

        {spaces.map((space) => {
          const spaceTabs = unpinned.filter((t) => t.spaceId === space.id);
          return (
            <div className="space" key={space.id}>
              <div
                className={
                  "space__head" + (dropTarget === space.id ? " drop-target" : "")
                }
                onDragOver={(e) => {
                  e.preventDefault();
                  setDropTarget(space.id);
                }}
                onDragLeave={() => setDropTarget(null)}
                onDrop={(e) => onDropOnSpace(e, space.id)}
              >
                <span className="space__name">{space.name}</span>
                <button
                  className="space__bind"
                  type="button"
                  title={space.roomId ? "Refresh room" : "Bind room"}
                  onClick={() =>
                    space.roomId
                      ? void actions.refreshRoom(space.id)
                      : void actions.bindSpaceToRoom(space.id)
                  }
                >
                  {space.roomId ? "Room" : "Bind"}
                </button>
              </div>
              <div className="tablist" role="tablist" aria-label={space.name}>
                {renderList(spaceTabs)}
              </div>
            </div>
          );
        })}
      </div>

      <div className="sidebar__foot">
        <button
          className="iconbtn"
          onClick={() => actions.addSpace(`Space ${state.spaces.length + 1}`)}
          title="New Space"
        >
          <PlusIcon size={15} />
        </button>
        <span className="newtab__hint" style={{ flex: 1 }}>
          New Space
        </span>
        <button className="iconbtn" onClick={actions.toggleRail} title="Toggle chat rail">
          <PanelIcon />
        </button>
        <button
          className="iconbtn"
          onClick={() => {
            actions.setQueuePanelOpen(!state.queuePanelOpen);
            if (!state.queuePanelOpen) void actions.refreshQueue();
          }}
          title="Queue"
        >
          Q
        </button>
      </div>
      {state.queuePanelOpen && (
        <div className="queue-panel">
          {state.queueJobs.length === 0 ? (
            <div className="queue-panel__empty">No jobs</div>
          ) : (
            state.queueJobs.map((job) => (
              <div className="queue-row" key={job.jobId}>
                <span>{job.title}</span>
                <code>{job.status}</code>
              </div>
            ))
          )}
        </div>
      )}
    </aside>
  );
}
