import { useApp } from "./state/store";
import { Sidebar } from "./components/Sidebar";
import { Omnibox } from "./components/Omnibox";
import { WebviewStage } from "./components/WebviewStage";
import { ChatRail } from "./components/ChatRail";
import { Settings } from "./components/Settings";
import { PreActionPreview } from "./components/PreActionPreview";
import { AGENT_SURFACE_ENABLED } from "./lib/flags";

export default function App() {
  const { state } = useApp();
  return (
    <div className="app">
      <div className={"shell" + (state.railVisible ? " shell--rail" : "")}>
        <Sidebar />
        <main className="main">
          <Omnibox />
          <WebviewStage />
        </main>
        {state.railVisible && <ChatRail />}
      </div>
      {state.settingsOpen && <Settings />}
      {AGENT_SURFACE_ENABLED && (
        <div className="preaction-host">
          <PreActionPreview />
        </div>
      )}
    </div>
  );
}
