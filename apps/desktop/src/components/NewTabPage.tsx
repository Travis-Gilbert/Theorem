import { useEffect, useRef, useState } from "react";
import { useApp } from "../state/store";
import { GlobeIcon } from "./icons";

/** Ask-first new-tab page (D2): a focused input, recent tabs and Spaces below. */
export function NewTabPage() {
  const { state, actions } = useApp();
  const [value, setValue] = useState("");
  const ref = useRef<HTMLInputElement>(null);

  useEffect(() => {
    ref.current?.focus();
  }, []);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!value.trim()) return;
    await actions.submitOmnibox(value);
    setValue("");
  };

  const recent = state.tabs
    .filter((t) => t.kind === "web")
    .slice(-8)
    .reverse();

  return (
    <div className="newtab">
      <div className="newtab__brand">
        <span className="brand__mark" /> Theorem
      </div>

      <form className="newtab__ask" onSubmit={submit}>
        <input
          ref={ref}
          className="newtab__askbox"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder="Ask anything, or enter a URL"
          spellCheck={false}
          autoComplete="off"
        />
        <div className="newtab__hint">
          Type a URL to navigate, or a question to start a chat in the rail.
        </div>
      </form>

      {recent.length > 0 && (
        <div className="newtab__section">
          <div className="section__label">Recent tabs</div>
          <div className="newtab__grid">
            {recent.map((t) => (
              <button
                key={t.id}
                className="recent"
                onClick={() => actions.selectTab(t.id)}
                title={t.url}
              >
                <span className="tab__favicon">
                  {t.favicon ? <img src={t.favicon} alt="" /> : <GlobeIcon size={14} />}
                </span>
                <span className="recent__title">{t.title || t.url}</span>
              </button>
            ))}
          </div>
        </div>
      )}

      {state.spaces.length > 0 && (
        <div className="newtab__section">
          <div className="section__label">Spaces</div>
          <div className="newtab__grid">
            {state.spaces.map((s) => (
              <div key={s.id} className="recent">
                <span className="recent__title">{s.name}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
