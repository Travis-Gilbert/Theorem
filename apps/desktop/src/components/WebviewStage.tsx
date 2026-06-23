import { useEffect, useRef } from "react";
import { useApp } from "../state/store";
import * as cmd from "../lib/commands";
import { isTauri } from "../lib/commands";
import { NewTabPage } from "./NewTabPage";

/**
 * The stage is the hole in the chrome where the active tab's wry webview shows.
 * The webview is a native layer the backend positions; here we only measure the
 * stage rect and report it via tab_set_bounds. In plain Vite mode (no Tauri)
 * the webview does not exist, so we render a placeholder instead.
 */
export function WebviewStage() {
  const { state, actions } = useApp();
  const active = state.tabs.find((t) => t.id === state.activeTabId);
  const showNewTab = !active || active.kind === "newtab";
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!isTauri()) return;
    const el = ref.current;
    if (!el) return;
    const report = () => {
      const r = el.getBoundingClientRect();
      void cmd.tabSetBounds({
        x: Math.round(r.left),
        y: Math.round(r.top),
        width: Math.round(r.width),
        height: Math.round(r.height),
      });
    };
    report();
    const ro = new ResizeObserver(report);
    ro.observe(el);
    window.addEventListener("resize", report);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", report);
    };
  }, [state.railVisible, state.activeTabId, showNewTab]);

  return (
    <div className="stage" ref={ref}>
      {showNewTab ? (
        <NewTabPage />
      ) : (
        <>
        {active?.kind === "agent" && (
          <div className="agent-strip">
            <span>Agent</span>
            <button type="button" onClick={() => void actions.ingestAgentTab(active.id)}>
              Ingest
            </button>
          </div>
        )}
        {!isTauri() && (
          <div className="stage__placeholder">
            <div>
              <div style={{ fontWeight: 600, color: "var(--text)" }}>
                {active?.title || active?.url}
              </div>
              <div>Web content renders in a native webview in the desktop build.</div>
            </div>
          </div>
        )}
        </>
      )}
    </div>
  );
}
