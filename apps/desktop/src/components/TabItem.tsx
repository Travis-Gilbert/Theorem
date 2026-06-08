import type { Tab } from "../state/types";
import { CloseIcon, GlobeIcon, PinIcon } from "./icons";

interface Props {
  tab: Tab;
  active: boolean;
  /** Agent-tab ingestion receipt count (D4.2); shown only on agent tabs. */
  badge?: number;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onTogglePin: (id: string) => void;
  onDropOnTab: (draggedId: string, targetId: string) => void;
  /** Roving-tabindex keyboard handler, owned by the Sidebar (D3.3). */
  onKeyNav: (tabId: string, e: React.KeyboardEvent<HTMLDivElement>) => void;
  registerRef: (id: string, el: HTMLDivElement | null) => void;
}

export function TabItem({
  tab,
  active,
  badge,
  onSelect,
  onClose,
  onTogglePin,
  onDropOnTab,
  onKeyNav,
  registerRef,
}: Props) {
  return (
    <div
      ref={(el) => registerRef(tab.id, el)}
      className={"tab" + (active ? " tab--active" : "")}
      role="tab"
      aria-selected={active}
      tabIndex={active ? 0 : -1}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData("text/tab-id", tab.id);
        e.dataTransfer.effectAllowed = "move";
      }}
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => {
        e.preventDefault();
        e.stopPropagation();
        const id = e.dataTransfer.getData("text/tab-id");
        if (id && id !== tab.id) onDropOnTab(id, tab.id);
      }}
      onClick={() => onSelect(tab.id)}
      onKeyDown={(e) => onKeyNav(tab.id, e)}
      title={tab.url || tab.title}
    >
      <span className="tab__favicon">
        {tab.favicon ? <img src={tab.favicon} alt="" /> : <GlobeIcon size={14} />}
      </span>
      <span className="tab__title">{tab.title || tab.url || "New Tab"}</span>
      {tab.kind === "agent" && badge ? (
        <span className="tab__badge" title={`${badge} ingested this session`}>{badge}</span>
      ) : null}
      <button
        className={"tab__pin" + (tab.pinned ? " tab__pin--on" : "")}
        onClick={(e) => {
          e.stopPropagation();
          onTogglePin(tab.id);
        }}
        title={tab.pinned ? "Unpin" : "Pin"}
        tabIndex={-1}
      >
        <PinIcon size={13} />
      </button>
      <button
        className="tab__close"
        onClick={(e) => {
          e.stopPropagation();
          onClose(tab.id);
        }}
        title="Close tab"
        tabIndex={-1}
      >
        <CloseIcon size={13} />
      </button>
    </div>
  );
}
