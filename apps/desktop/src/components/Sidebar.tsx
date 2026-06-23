import { useState } from "react";
import { useApp } from "../state/store";
import type { SpaceId } from "../state/types";
import { TabItem } from "./TabItem";
import {
  AgentIcon,
  GearIcon,
  PanelIcon,
  PlusIcon,
  QueueIcon,
  ReviewIcon,
  RouteIcon,
  SourceIcon,
  TaskIcon,
} from "./icons";

export function Sidebar() {
  const { state, actions } = useApp();
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  const pinned = state.tabs.filter((t) => t.pinned);
  const unpinned = state.tabs.filter((t) => !t.pinned);
  const ungrouped = unpinned.filter((t) => !t.spaceId);
  const spaces = [...state.spaces].sort((a, b) => a.order - b.order);

  // Move dragged tab to sit before the target tab, inheriting the target's space.
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
        onSelect={actions.selectTab}
        onClose={actions.closeTab}
        onTogglePin={actions.togglePin}
        onDragStartId={() => undefined}
        onDropOnTab={onDropOnTab}
      />
    ));

  return (
    <aside className="sidebar">
      <div className="sidebar__head">
        <span className="brand">
          <span className="brand__mark" />
          CommonPlace
        </span>
        <span className="sidebar__spacer" />
        <button className="iconbtn" onClick={actions.newTab} title="New intake view">
          <PlusIcon />
        </button>
        <button className="iconbtn" onClick={() => void actions.newAgentTab()} title="New agent review">
          <AgentIcon />
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
        <div className="side-nav" aria-label="CommonPlace areas">
          <button type="button" className="side-nav__item side-nav__item--active">
            <ReviewIcon size={14} />
            <span>Review Queue</span>
            <code aria-label="Metric pending">-</code>
          </button>
          <button type="button" className="side-nav__item">
            <SourceIcon size={14} />
            <span>Sources</span>
            <code aria-label="Metric pending">-</code>
          </button>
          <button type="button" className="side-nav__item">
            <RouteIcon size={14} />
            <span>Routed</span>
            <code aria-label="Metric pending">-</code>
          </button>
          <button type="button" className="side-nav__item">
            <TaskIcon size={14} />
            <span>Tasks</span>
            <code aria-label="Metric pending">-</code>
          </button>
        </div>

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
          <div className="section__label">Open Surfaces</div>
          <div
            className={"tablist" + (dropTarget === "ungrouped" ? " drop-target" : "")}
            onDragOver={(e) => {
              e.preventDefault();
              setDropTarget("ungrouped");
            }}
            onDragLeave={() => setDropTarget(null)}
            onDrop={(e) => onDropOnSpace(e, undefined)}
          >
            {ungrouped.length === 0 ? (
              <div className="newtab__hint" style={{ padding: "4px 8px" }}>
                No open browser surfaces.
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
                  title={space.roomId ? "Refresh coordination" : "Connect coordination"}
                  onClick={() =>
                    space.roomId
                      ? void actions.refreshRoom(space.id)
                      : void actions.bindSpaceToRoom(space.id)
                  }
                >
                  {space.roomId ? "Synced" : "Connect"}
                </button>
              </div>
              <div className="tablist">{renderList(spaceTabs)}</div>
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
          title="Review jobs"
        >
          <QueueIcon size={15} />
        </button>
      </div>
      {state.queuePanelOpen && (
        <div className="queue-panel">
          {state.queueJobs.length === 0 ? (
            <div className="queue-panel__empty">No review jobs</div>
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
