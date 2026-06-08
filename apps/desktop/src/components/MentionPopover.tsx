import type { Tab } from "../state/types";

interface Props {
  tabs: Tab[];
  activeIndex: number;
  onPick: (tab: Tab) => void;
}

/** The @-mention candidate list shown above the composer (D4 / APG listbox). */
export function MentionPopover({ tabs, activeIndex, onPick }: Props) {
  if (tabs.length === 0) return null;
  return (
    <div className="mention-pop" id="mention-listbox" role="listbox" aria-label="Mention a tab">
      {tabs.map((t, i) => (
        <div
          key={t.id}
          id={`mention-opt-${i}`}
          role="option"
          aria-selected={i === activeIndex}
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
