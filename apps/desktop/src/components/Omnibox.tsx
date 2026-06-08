import { useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../state/store";
import { routeOmnibox } from "../lib/routing";
import * as cmd from "../lib/commands";
import type { Tab } from "../state/types";
import { BackIcon, ForwardIcon, GlobeIcon, PanelIcon, ReloadIcon } from "./icons";

interface Suggestion {
  id: string;
  kind: "route" | "tab";
  label: string;
  sub?: string;
  tab?: Tab;
}

export function Omnibox() {
  const { state, actions } = useApp();
  const active = state.tabs.find((t) => t.id === state.activeTabId);
  const isWeb = active?.kind === "web";
  const [value, setValue] = useState("");
  const [open, setOpen] = useState(false);
  const [idx, setIdx] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);

  // Reflect the active web tab's URL; clear for the ask-first new-tab page.
  useEffect(() => {
    setValue(active && active.kind === "web" ? active.url : "");
  }, [active?.id, active?.url, active?.kind]);

  // Cmd/Ctrl+L focuses and selects the omnibox.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "l") {
        e.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Suggestions: the shape-routed action first, then matching open web tabs.
  const suggestions = useMemo<Suggestion[]>(() => {
    const v = value.trim();
    if (!v) return [];
    const route = routeOmnibox(v);
    const list: Suggestion[] = [
      route.kind === "navigate"
        ? { id: "route", kind: "route", label: `Go to ${route.url}`, sub: "Enter" }
        : { id: "route", kind: "route", label: `Ask: ${route.text}`, sub: "rail" },
    ];
    const q = v.toLowerCase();
    for (const t of state.tabs) {
      if (t.id === state.activeTabId || t.kind !== "web") continue;
      if (`${t.title} ${t.url}`.toLowerCase().includes(q)) {
        list.push({ id: t.id, kind: "tab", label: t.title || t.url, sub: t.url, tab: t });
      }
      if (list.length >= 7) break;
    }
    return list;
  }, [value, state.tabs, state.activeTabId]);

  const commit = (s: Suggestion) => {
    if (s.kind === "tab" && s.tab) {
      void actions.selectTab(s.tab.id);
    } else {
      const route = routeOmnibox(value);
      void actions.submitOmnibox(value);
      if (route.kind === "chat") setValue("");
    }
    setOpen(false);
    setIdx(-1);
  };

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (open && idx >= 0 && suggestions[idx]) {
      commit(suggestions[idx]);
      return;
    }
    const route = routeOmnibox(value);
    void actions.submitOmnibox(value);
    if (route.kind === "chat") setValue("");
    setOpen(false);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (!open && suggestions.length) {
        setOpen(true);
        setIdx(0);
      } else {
        setIdx((i) => Math.min(i + 1, suggestions.length - 1));
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (open) setIdx((i) => (i <= 0 ? -1 : i - 1));
    } else if (e.key === "Escape") {
      if (open) {
        e.preventDefault();
        setOpen(false);
        setIdx(-1);
      } else {
        inputRef.current?.blur();
      }
    }
  };

  return (
    <form className="omnibox" onSubmit={submit} role="search">
      <div className="omnibox__nav">
        <button
          type="button"
          className="iconbtn"
          disabled={!isWeb || !active?.canGoBack}
          onClick={() => active && cmd.tabGoBack(active.id)}
          title="Back"
        >
          <BackIcon />
        </button>
        <button
          type="button"
          className="iconbtn"
          disabled={!isWeb || !active?.canGoForward}
          onClick={() => active && cmd.tabGoForward(active.id)}
          title="Forward"
        >
          <ForwardIcon />
        </button>
        <button
          type="button"
          className="iconbtn"
          disabled={!isWeb}
          onClick={() => active && cmd.tabReload(active.id)}
          title="Reload"
        >
          <ReloadIcon />
        </button>
      </div>

      <div className="omnibox__field">
        <input
          ref={inputRef}
          className="omnibox__input"
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
            setOpen(true);
            setIdx(-1);
          }}
          onFocus={() => {
            if (value.trim()) setOpen(true);
          }}
          onBlur={() => setOpen(false)}
          onKeyDown={onKeyDown}
          placeholder="Search, enter a URL, or ask anything"
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          role="combobox"
          aria-expanded={open && suggestions.length > 0}
          aria-controls="omnibox-listbox"
          aria-autocomplete="list"
          aria-activedescendant={open && idx >= 0 ? `omnibox-opt-${idx}` : undefined}
        />
        {open && suggestions.length > 0 && (
          <ul className="omnibox__suggestions" id="omnibox-listbox" role="listbox">
            {suggestions.map((s, i) => (
              <li
                key={s.id}
                id={`omnibox-opt-${i}`}
                role="option"
                aria-selected={i === idx}
                className={
                  "omnibox__suggestion" + (i === idx ? " omnibox__suggestion--active" : "")
                }
                // onMouseDown (not click) so the input does not blur before commit.
                onMouseDown={(e) => {
                  e.preventDefault();
                  commit(s);
                }}
                onMouseEnter={() => setIdx(i)}
              >
                <span className="omnibox__suggestion-icon">
                  {s.kind === "tab" ? <GlobeIcon size={14} /> : null}
                </span>
                <span className="omnibox__suggestion-label">{s.label}</span>
                {s.sub && <span className="omnibox__suggestion-sub">{s.sub}</span>}
              </li>
            ))}
          </ul>
        )}
      </div>

      <button
        type="button"
        className="iconbtn"
        onClick={actions.toggleRail}
        title="Toggle chat rail"
      >
        <PanelIcon />
      </button>
    </form>
  );
}
