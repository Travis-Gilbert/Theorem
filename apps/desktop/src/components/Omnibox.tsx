import { useEffect, useRef, useState } from "react";
import { useApp } from "../state/store";
import { routeOmnibox } from "../lib/routing";
import * as cmd from "../lib/commands";
import { BackIcon, ForwardIcon, PanelIcon, ReloadIcon } from "./icons";

export function Omnibox() {
  const { state, actions } = useApp();
  const active = state.tabs.find((t) => t.id === state.activeTabId);
  const isWeb = active?.kind === "web";
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Reflect the active web tab's URL; clear for the ask-first new-tab page.
  useEffect(() => {
    setValue(active && active.kind === "web" ? active.url : "");
  }, [active?.id, active?.url, active?.kind]);

  // Cmd/Ctrl+L focuses the omnibox.
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

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const v = value;
    const route = routeOmnibox(v);
    await actions.submitOmnibox(v);
    if (route.kind === "chat") setValue("");
  };

  return (
    <form className="omnibox" onSubmit={submit}>
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
      <input
        ref={inputRef}
        className="omnibox__input"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        placeholder="Search, enter a URL, or ask anything"
        spellCheck={false}
        autoComplete="off"
        autoCorrect="off"
      />
      <button
        type="button"
        className={"iconbtn" + (state.railVisible ? "" : "")}
        onClick={actions.toggleRail}
        title="Toggle chat rail"
      >
        <PanelIcon />
      </button>
    </form>
  );
}
