import type { Tab } from "../state/types";
import { CloseIcon, GlobeIcon, PinIcon } from "./icons";

interface Props {
  tab: Tab;
  active: boolean;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onTogglePin: (id: string) => void;
  onDragStartId: (id: string) => void;
  onDropOnTab: (draggedId: string, targetId: string) => void;
}

export function TabItem({
  tab,
  active,
  onSelect,
  onClose,
  onTogglePin,
  onDragStartId,
  onDropOnTab,
}: Props) {
  return (
    <div
      className={"tab" + (active ? " tab--active" : "")}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData("text/tab-id", tab.id);
        e.dataTransfer.effectAllowed = "move";
        onDragStartId(tab.id);
      }}
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => {
        e.preventDefault();
        e.stopPropagation();
        const id = e.dataTransfer.getData("text/tab-id");
        if (id && id !== tab.id) onDropOnTab(id, tab.id);
      }}
      onClick={() => onSelect(tab.id)}
      title={tab.url || tab.title}
    >
      <span className="tab__favicon">
        {tab.favicon ? <img src={tab.favicon} alt="" /> : <GlobeIcon size={14} />}
      </span>
      <span className="tab__title">{tab.title || tab.url || "New Tab"}</span>
      <button
        className={"tab__pin" + (tab.pinned ? " tab__pin--on" : "")}
        onClick={(e) => {
          e.stopPropagation();
          onTogglePin(tab.id);
        }}
        title={tab.pinned ? "Unpin" : "Pin"}
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
      >
        <CloseIcon size={13} />
      </button>
    </div>
  );
}
