"use client";

import * as React from "react";
import {
  AlertTriangle,
  Archive,
  Boxes,
  Brain,
  ChevronLeft,
  ChevronRight,
  Clock3,
  Folder,
  GitBranch,
  Globe,
  Grid3X3,
  Library,
  Map,
  Paperclip,
  PenLine,
  Search,
  Send,
  SlidersHorizontal,
  Sparkles,
  WandSparkles,
} from "lucide-react";
import { HARNESS_OBJECT_SETS, mockBlockHost } from "@/lib/block-view/harness-fixtures";
import { HARNESS_VIEW_DESCRIPTORS } from "@/lib/block-view/harness-registry";
import { ViewRegistry } from "@/lib/block-view";
import type { ObjectSet, ViewDescriptor } from "@/lib/block-view";

const registry = new ViewRegistry(HARNESS_VIEW_DESCRIPTORS);

export function CodeWorkspaceShell() {
  return (
    <main className="commonplace-workspace-theme cpw-shell">
      <CommonPlaceSidebar />

      <section className="cpw-stage" aria-label="CommonPlace block workspace">
        <aside className="cpw-sources-column" aria-label="Sources">
          <SectionLabel label="Sources" />
          <SourceRows />
          <div className="cpw-sidebar-block">
            <BlockSlot viewId="file-tree" set={HARNESS_OBJECT_SETS.files} />
          </div>
          <div className="cpw-sidebar-block cpw-terminal-slot">
            <BlockSlot viewId="terminal" set={HARNESS_OBJECT_SETS.terminal} />
          </div>
          <ProgressFooter />
        </aside>

        <section className="cpw-center-column" aria-label="Needs you">
          <header className="cpw-day-header">
            <div className="cpw-day-label">
              <span className="cpw-pulse-dot" />
              <span>Thursday, June 25</span>
            </div>
            <p>Harness object blocks are ready for review inside the CommonPlace surface.</p>
          </header>

          <section className="cpw-review-area">
            <div className="cpw-toolbar-row">
              <div>
                <SectionLabel label="Needs You" />
                <p>Items below the confidence line, ready for one action.</p>
              </div>
              <span className="cpw-count-pill">2</span>
            </div>

            <div className="cpw-primary-grid">
              <BlockSlot viewId="code-editor" set={HARNESS_OBJECT_SETS.files} />
              <BlockSlot viewId="patch-review" set={HARNESS_OBJECT_SETS.patch} />
            </div>

            <BlockSlot viewId="agent-run-board" set={HARNESS_OBJECT_SETS.runs} />
          </section>
        </section>

        <aside className="cpw-organized-column" aria-label="Organized today">
          <div className="cpw-proof-head">
            <SectionLabel label="Organized Today" />
            <p>Automatic filing, recent routes, and where the engine is putting things.</p>
          </div>

          <div className="cpw-proof-stack">
            <BlockSlot viewId="agent-thread" set={HARNESS_OBJECT_SETS.thread} />
            <BlockSlot viewId="run-trace" set={HARNESS_OBJECT_SETS.trace} />
            <div className="cpw-side-grid">
              <BlockSlot viewId="tool-activity" set={HARNESS_OBJECT_SETS.tools} />
              <BlockSlot viewId="context-artifact" set={HARNESS_OBJECT_SETS.context} />
            </div>
          </div>

          <AgentDock />
        </aside>
      </section>
    </main>
  );
}

function CommonPlaceSidebar() {
  return (
    <aside className="cpw-sidebar" aria-label="CommonPlace navigation">
      <div className="cpw-sidebar-glow" aria-hidden />
      <div className="cpw-brand-row">
        <div className="cpw-brand">
          <span>Common</span>
          <strong>Place</strong>
        </div>
        <button className="cpw-icon-button" type="button" aria-label="Collapse sidebar">
          <ChevronLeft size={16} />
        </button>
      </div>

      <label className="cpw-search">
        <Search size={17} />
        <span>Search, capture, or / for commands</span>
      </label>

      <nav className="cpw-nav" aria-label="CommonPlace sections">
        <NavItem icon={Archive} label="Auto Organize" active section="capture" />
        <NavItem icon={Grid3X3} label="Library" section="capture" />
        <NavItem icon={Brain} label="Models" section="capture" count=">" />
        <NavItem icon={WandSparkles} label="Artifacts" active section="capture" marker />
        <NavItem icon={PenLine} label="Compose" section="capture" />
        <div className="cpw-tree-row">
          <ChevronRight size={14} />
          <Folder size={18} />
          <span>Files</span>
          <Boxes size={16} className="cpw-tree-action" />
        </div>

        <NavSection label="Views" />
        <NavItem icon={Clock3} label="Timeline" section="views" count="3" />
        <NavItem icon={Map} label="Map" section="views" />

        <NavSection label="Work" />
        <NavItem icon={Library} label="Notebooks" section="work" count="1" />
        <NavItem icon={GitBranch} label="Projects" section="work" count="0" />

        <NavSection label="System" />
        <NavItem icon={Sparkles} label="Agents" section="system" count="4" />
        <NavItem icon={SlidersHorizontal} label="Desktop" section="system" count="3" />
      </nav>

      <button className="cpw-engine-status" type="button">
        <span className="cpw-engine-dot" />
        <span>Engine</span>
      </button>
    </aside>
  );
}

function NavSection({ label }: { label: string }) {
  return (
    <>
      <div className="cpw-sidebar-divider" />
      <div className="cpw-nav-section">{label}</div>
    </>
  );
}

function NavItem({
  icon: Icon,
  label,
  active,
  marker,
  count,
  section,
}: {
  icon: React.ComponentType<{ size?: number; className?: string }>;
  label: string;
  active?: boolean;
  marker?: boolean;
  count?: string;
  section: "capture" | "views" | "work" | "system";
}) {
  return (
    <button className="cpw-nav-item" data-active={active ? "true" : "false"} data-section={section} type="button">
      <Icon size={17} />
      <span>{label}</span>
      {marker ? <span className="cpw-nav-marker" /> : null}
      {count ? <span className="cpw-nav-count">{count}</span> : null}
    </button>
  );
}

function SectionLabel({ label }: { label: string }) {
  return (
    <div className="cpw-section-label">
      <span />
      <strong>{label}</strong>
    </div>
  );
}

function SourceRows() {
  return (
    <div className="cpw-source-list" aria-label="Source groups">
      {[
        ["Emails", "0", "blue"],
        ["Notes", "0", "gold"],
        ["Files", "4", "teal"],
        ["Tasks", "1", "orange"],
      ].map(([label, count, tone]) => (
        <button className="cpw-source-row" data-tone={tone} key={label} type="button">
          <span className="cpw-source-dot" />
          <span>{label}</span>
          <small>{count}</small>
        </button>
      ))}
    </div>
  );
}

function ProgressFooter() {
  return (
    <div className="cpw-progress-footer">
      <div className="cpw-progress-rule" />
      <div className="cpw-progress-meta">
        <span>Progress</span>
        <strong>0 of 2 done</strong>
      </div>
      <div className="cpw-progress-track">
        <span />
      </div>
      <div className="cpw-segmented">
        <button type="button" data-active="true">Day</button>
        <button type="button">Week</button>
        <button type="button">Month</button>
      </div>
    </div>
  );
}

function AgentDock() {
  return (
    <form className="cpw-agent-dock" aria-label="Ask the Theorem agent">
      <label htmlFor="cpw-agent-input">Ask the Theorem agent</label>
      <input id="cpw-agent-input" aria-label="Ask the Theorem agent" />
      <div className="cpw-agent-tools">
        <button type="button" aria-label="Attach file">
          <Paperclip size={21} />
        </button>
        <button type="button" aria-label="Search the web">
          <Globe size={21} />
        </button>
        <button type="button" aria-label="Use agent tools">
          <Sparkles size={21} />
        </button>
        <button type="button" aria-label="Open graph context">
          <GitBranch size={21} />
        </button>
        <button className="cpw-send-button" type="submit" aria-label="Send">
          <Send size={20} />
        </button>
      </div>
    </form>
  );
}

function BlockSlot({ viewId, set }: { viewId: string; set: ObjectSet }) {
  const view = React.useMemo(() => pickView(viewId, set), [set, viewId]);

  if (!view) {
    return (
      <div className="cpw-missing-block">
        <AlertTriangle size={14} className="mr-2" />
        No descriptor matched {viewId}
      </div>
    );
  }

  const View = view.render;
  return <View set={set} host={mockBlockHost} />;
}

function pickView(viewId: string, set: ObjectSet): ViewDescriptor | undefined {
  return registry.viewsFor(set.shape).find((view) => view.id === viewId);
}
