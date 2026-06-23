import type { Tab } from "../state/types";

interface Props {
  tabs: Tab[];
  activeIndex: number;
  onPick: (tab: Tab) => void;
}

/** The @-mention candidate list shown above the composer (D4). */
export function MentionPopover({ tabs, activeIndex, onPick }: Props) {
  if (tabs.length === 0) return null;
  return (
    <div className="mention-pop">
      {tabs.map((t, i) => (
        <div
          key={t.id}
          className={"mention-item" + (i === activeIndex ? " mention-item--active" : "")}
          // onMouseDown (not onClick) so the textarea does not blur before pick.
          onMouseDown={(e) => {
            e.preventDefault();
            onPick(t);
          }}
        >
          <span className="mention-item__title">{t.title || t.url}</span>
          <span className="mention-item__url">{t.url}</span>
        </div>
      ))}
    </div>
  );
}
