"use client";

import * as React from "react";
import {
  ArrowUp,
  BookOpen,
  Boxes,
  Bell,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  CircleDot,
  Clock3,
  Code2,
  Command,
  CreditCard,
  Cpu,
  Database,
  FileText,
  Folder,
  FolderPlus,
  GitBranch,
  Globe,
  Gauge,
  History,
  Inbox,
  Layers3,
  Link2,
  ListChecks,
  Map,
  MessageSquareText,
  MonitorCog,
  Paperclip,
  PenLine,
  Plus,
  Route,
  Search,
  SlidersHorizontal,
  Settings,
  Scissors,
  Sparkles,
  SquareTerminal,
  Table2,
  Upload,
  UserCircle,
  WandSparkles,
  Workflow,
} from "lucide-react";
import {
  AssistantRuntimeProvider,
  ComposerPrimitive,
  MessagePartPrimitive,
  MessagePrimitive,
  ThreadPrimitive,
  useExternalStoreRuntime,
  type AppendMessage,
  type MessageState,
  type ThreadAssistantMessagePart,
  type ThreadMessage,
  type ThreadUserMessagePart,
} from "@assistant-ui/react";
import CodeMirrorMerge from "react-codemirror-merge";
import { DotMatrix } from "@/components/assistant-ui/dot-matrix";
import { CosmosGraph } from "@/components/graph/CosmosGraph";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  omnibarIconButtonClass,
  omnibarRowClass,
  omnibarSendButtonClass,
  omnibarSurfaceClass,
} from "@/components/island/OmnibarChrome";
import {
  COMMONPLACE_ACCOUNT_ITEMS,
  COMMONPLACE_DATA_VIEW_DESCRIPTORS,
  COMMONPLACE_DATA_VIEWS,
  COMMONPLACE_OMNIBAR_CAPABILITIES,
  COMMONPLACE_SCENE_RENDERERS,
  COMMONPLACE_TOOLBOX,
  COMMONPLACE_WORK_PAGES,
  type CommonplaceIaItem,
  type CommonplaceDataViewDescriptor,
  type CommonplaceToolboxGroup,
} from "@/lib/commonplace/information-architecture";
import {
  dotStateForApiModelState,
  type ApiModelState,
  type CodeAgentModelStatus,
  type CodeDiffArtifact,
} from "@/lib/commonplace/code-agent-contract";
import {
  CODE_AGENT_TRANSPORTS,
  runCodeAgentTurn,
  type CodeAgentTransportId,
} from "@/lib/commonplace/code-agent-transport";
import {
  normalizeCommonplaceRustyRedViewId,
  type CommonplaceRustyRedDataPayload,
} from "@/lib/commonplace/rustyred-data-contract";
import { useCommonplaceRustyRedData } from "@/lib/commonplace/rustyred-data-client";
import { createMersenneBinaryGlyphs } from "@/lib/commonplace/mersenne-pattern";
import {
  commonplaceCodeExtensions,
  commonplaceCodeMirrorTheme,
  readOnlyExtensions,
} from "./commonplace-code-theme";

const MergeOriginal = CodeMirrorMerge.Original;
const MergeModified = CodeMirrorMerge.Modified;
type SidebarIcon = React.ComponentType<{ size?: number; className?: string }>;
type CommonPlaceSurfaceId =
  | "index"
  | "threads"
  | "write"
  | "code"
  | "artifacts"
  | "files"
  | "graph"
  | "table"
  | "map"
  | "timeline"
  | "clips"
  | "account"
  | "agents"
  | "engine"
  | "desktop"
  | "settings";

const INITIAL_ASSISTANT_ID = "code-agent:assistant:intro";
const INITIAL_DATE = new Date("2026-06-25T00:00:00.000Z");
const DEFAULT_COMMONPLACE_SURFACE: CommonPlaceSurfaceId = "index";

const DEFAULT_MODEL_STATUSES: readonly CodeAgentModelStatus[] = [
  { id: "router", label: "Router", state: "idle" },
  { id: "planner", label: "Planner", state: "idle" },
  { id: "editor", label: "Editor", state: "idle" },
  { id: "reviewer", label: "Reviewer", state: "idle" },
] as const;

const WORK_ICONS: Record<string, SidebarIcon> = {
  index: Inbox,
  threads: MessageSquareText,
  write: PenLine,
  code: Code2,
  artifacts: WandSparkles,
};

const DATA_VIEW_ICONS: Record<string, SidebarIcon> = {
  files: Folder,
  graph: GitBranch,
  table: Table2,
  map: Map,
  timeline: Clock3,
  clips: Scissors,
};

const CAPABILITY_ICONS: Record<string, SidebarIcon> = {
  "instant-kg": Layers3,
  web: Globe,
  attach: Paperclip,
  tier: Gauge,
  "git-aware": GitBranch,
  deepen: Sparkles,
};

const TOOLBOX_ICONS: Record<string, SidebarIcon> = {
  terminal: SquareTerminal,
  cluster: Boxes,
  timeline: Clock3,
  note: PenLine,
  task: Plus,
  reminder: Bell,
  project: GitBranch,
};

const ACCOUNT_ICONS: Record<string, SidebarIcon> = {
  account: UserCircle,
  agents: Sparkles,
  engine: Cpu,
  desktop: MonitorCog,
  settings: Settings,
};

const QUICK_ACTION_ICONS: Record<string, SidebarIcon> = {
  terminal: SquareTerminal,
  cluster: Boxes,
  timeline: Clock3,
  note: PenLine,
  task: ListChecks,
  reminder: Bell,
  project: GitBranch,
};

const SURFACE_LABELS: Record<CommonPlaceSurfaceId, string> = {
  index: "Index",
  threads: "Threads",
  write: "Write",
  code: "Code",
  artifacts: "Artifacts",
  files: "Files",
  graph: "Graph",
  table: "Table",
  map: "Map",
  timeline: "Timeline",
  clips: "Clips",
  account: "Account",
  agents: "Agents",
  engine: "Engine",
  desktop: "Desktop",
  settings: "Settings",
};

const DATA_VIEW_IDS = new Set(COMMONPLACE_DATA_VIEWS.map((item) => item.id));
const ACCOUNT_SURFACE_IDS = new Set(COMMONPLACE_ACCOUNT_ITEMS.map((item) => item.id));
const COMMONPLACE_SURFACE_IDS = new Set<CommonPlaceSurfaceId>([
  "index",
  "threads",
  "write",
  "code",
  "artifacts",
  "files",
  "graph",
  "table",
  "map",
  "timeline",
  "clips",
  "account",
  "agents",
  "engine",
  "desktop",
  "settings",
]);

function surfaceFromHash(hash: string): CommonPlaceSurfaceId | null {
  const candidate = hash.replace(/^#\/?/, "").trim().toLowerCase();
  return COMMONPLACE_SURFACE_IDS.has(candidate as CommonPlaceSurfaceId) ? (candidate as CommonPlaceSurfaceId) : null;
}

export function CodeWorkspaceShell() {
  const [activeSurface, setActiveSurface] = React.useState<CommonPlaceSurfaceId>(DEFAULT_COMMONPLACE_SURFACE);
  const [paletteOpen, setPaletteOpen] = React.useState(false);
  const selectSurface = React.useCallback((surface: CommonPlaceSurfaceId) => {
    setActiveSurface(surface);
    if (typeof window !== "undefined") {
      window.history.replaceState(null, "", `#${surface}`);
    }
  }, []);

  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((current) => !current);
      }
      if (event.key === "Escape") {
        setPaletteOpen(false);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  React.useEffect(() => {
    const syncSurfaceFromHash = () => {
      const surface = surfaceFromHash(window.location.hash);
      if (surface) setActiveSurface(surface);
    };

    syncSurfaceFromHash();
    window.addEventListener("hashchange", syncSurfaceFromHash);
    return () => window.removeEventListener("hashchange", syncSurfaceFromHash);
  }, []);

  return (
    <main className="commonplace-workspace-theme cpw-shell" data-merge-scope="preview-route">
      <CommonPlacePreviewSidebar
        activeSurface={activeSurface}
        onSelectSurface={selectSurface}
        onOpenPalette={() => setPaletteOpen(true)}
      />
      <CommonPlacePreviewStage
        activeSurface={activeSurface}
        onSelectSurface={selectSurface}
        paletteOpen={paletteOpen}
        onClosePalette={() => setPaletteOpen(false)}
      />
    </main>
  );
}

// Production CommonPlace should compose this stage inside its own app shell.
// The preview-only sidebar above exists only for this Theorem test route.
export function CodeWorkspaceStage() {
  return (
    <section className="cpw-stage cpw-code-agent-stage" aria-label="CommonPlace code workspace">
      <MersenneBinaryField />
      <CodeAgentRuntime />
    </section>
  );
}

function CommonPlacePreviewStage({
  activeSurface,
  onSelectSurface,
  paletteOpen,
  onClosePalette,
}: {
  activeSurface: CommonPlaceSurfaceId;
  onSelectSurface: (surface: CommonPlaceSurfaceId) => void;
  paletteOpen: boolean;
  onClosePalette: () => void;
}) {
  const isCode = activeSurface === "code";

  return (
    <section
      className={`cpw-stage ${isCode ? "cpw-code-agent-stage" : "cpw-ia-stage"}`}
      aria-label={`CommonPlace ${SURFACE_LABELS[activeSurface]} workspace`}
    >
      <MersenneBinaryField />
      {isCode ? (
        <CodeAgentRuntime />
      ) : (
        <>
          <CommonPlaceSurface surface={activeSurface} onSelectSurface={onSelectSurface} />
          <CommonPlaceAmbientOmnibar surface={activeSurface} onSelectSurface={onSelectSurface} />
        </>
      )}
      {paletteOpen ? <CommandPalettePreview onSelectSurface={onSelectSurface} onClose={onClosePalette} /> : null}
    </section>
  );
}

// Preview-only IA fixture for this Theorem test route.
// Do not move this component into the production CommonPlace app shell; only the
// plain IA data in `lib/commonplace/information-architecture.ts` is intended to
// survive the real merge.
function CommonPlacePreviewSidebar({
  activeSurface,
  onSelectSurface,
  onOpenPalette,
}: {
  activeSurface: CommonPlaceSurfaceId;
  onSelectSurface: (surface: CommonPlaceSurfaceId) => void;
  onOpenPalette: () => void;
}) {
  return (
    <aside className="cpw-sidebar" aria-label="CommonPlace preview navigation" data-merge-scope="preview-only">
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

      <OmnibarPreview onOpenPalette={onOpenPalette} />

      <nav className="cpw-nav" aria-label="CommonPlace sections">
        <NavSection label="Work" />
        {COMMONPLACE_WORK_PAGES.map((item) => (
          <NavItem
            key={item.id}
            item={item}
            icon={WORK_ICONS[item.id] ?? Folder}
            active={activeSurface === item.id}
            onSelect={() => onSelectSurface(item.id as CommonPlaceSurfaceId)}
          />
        ))}

        <NavSection label="Data" />
        {COMMONPLACE_DATA_VIEWS.map((item) => (
          item.id === "files" ? (
            <FilesTreeNavItem
              key={item.id}
              item={item}
              active={activeSurface === item.id}
              onSelect={() => onSelectSurface(item.id as CommonPlaceSurfaceId)}
            />
          ) : (
            <NavItem
              key={item.id}
              item={item}
              icon={DATA_VIEW_ICONS[item.id] ?? Database}
              active={activeSurface === item.id}
              onSelect={() => onSelectSurface(item.id as CommonPlaceSurfaceId)}
            />
          )
        ))}

        <ToolboxPreview groups={COMMONPLACE_TOOLBOX} />

        <NavSection label="Account" />
        {COMMONPLACE_ACCOUNT_ITEMS.map((item) => (
          <NavItem
            key={item.id}
            item={item}
            icon={ACCOUNT_ICONS[item.id] ?? SlidersHorizontal}
            active={activeSurface === item.id}
            onSelect={() => onSelectSurface(item.id as CommonPlaceSurfaceId)}
          />
        ))}
      </nav>

      <button className="cpw-engine-status" type="button">
        <span className="cpw-engine-dot" />
        <span>Engine</span>
      </button>
    </aside>
  );
}

function OmnibarPreview({ onOpenPalette }: { onOpenPalette: () => void }) {
  return (
    <section className="cpw-omnibar-preview" aria-label="Agent omnibar">
      <button className="cpw-search" type="button" aria-label="Ask the Theorem agent" onClick={onOpenPalette}>
        <Search size={17} />
        <span>Ask the Theorem agent</span>
        <small>
          <Command size={10} /> K
        </small>
      </button>
      <div className="cpw-capability-strip" aria-label="Agent capabilities">
        {COMMONPLACE_OMNIBAR_CAPABILITIES.map((capability) => {
          const Icon = CAPABILITY_ICONS[capability.id] ?? Sparkles;
          return (
            <button key={capability.id} className="cpw-capability-chip" type="button" title={capability.description}>
              <Icon size={12} />
              <span>{capability.label}</span>
            </button>
          );
        })}
      </div>
    </section>
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
  item,
  icon: Icon,
  active,
  onSelect,
}: {
  item: CommonplaceIaItem;
  icon: SidebarIcon;
  active?: boolean;
  onSelect?: () => void;
}) {
  return (
    <button
      className="cpw-nav-item"
      data-active={active ? "true" : "false"}
      data-placement={item.placement}
      type="button"
      title={item.description}
      aria-current={active ? "page" : undefined}
      onClick={onSelect}
    >
      <Icon size={17} />
      <span>{item.label}</span>
      {active ? <span className="cpw-nav-marker" /> : null}
      {item.count ? <span className="cpw-nav-count">{item.count}</span> : null}
    </button>
  );
}

function FilesTreeNavItem({
  item,
  active,
  onSelect,
}: {
  item: CommonplaceIaItem;
  active?: boolean;
  onSelect: () => void;
}) {
  const [open, setOpen] = React.useState(false);

  return (
    <div className="cpw-sidebar-file-tree" data-open={open ? "true" : "false"} data-active={active ? "true" : "false"}>
      <button
        className="cpw-tree-row cpw-files-tree-root"
        type="button"
        title={item.description}
        aria-expanded={open}
        onClick={() => {
          onSelect();
          setOpen(true);
        }}
      >
        <ChevronRight
          size={15}
          className="cpw-file-chevron"
          data-open={open ? "true" : "false"}
          onClick={(event) => {
            event.stopPropagation();
            setOpen((current) => !current);
          }}
        />
        <Folder size={19} />
        <span>{item.label}</span>
        {item.count ? <small className="cpw-file-tree-count">{item.count}</small> : null}
        <FolderPlus size={16} className="cpw-tree-action" aria-hidden />
      </button>
      <div className="cpw-file-tree-children" aria-hidden={!open}>
        <button className="cpw-file-tree-leaf" type="button" onClick={onSelect}>
          <span>Specs</span>
        </button>
        <button className="cpw-file-tree-leaf" type="button" onClick={onSelect}>
          <span>Uploads</span>
        </button>
      </div>
    </div>
  );
}

function ToolboxPreview({ groups }: { groups: readonly CommonplaceToolboxGroup[] }) {
  return (
    <details className="cpw-toolbox">
      <summary className="cpw-toolbox-summary">Toolbox</summary>
      <div className="cpw-toolbox-groups">
        {groups.map((group) => (
          <div key={group.id} className="cpw-toolbox-group">
            <span>{group.label}</span>
            <div>
              {group.items.map((item) => {
                const Icon = TOOLBOX_ICONS[item.id] ?? Plus;
                return (
                  <button key={item.id} className="cpw-toolbox-action" type="button" title={item.description}>
                    <Icon size={13} />
                    <span>{item.label}</span>
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </div>
    </details>
  );
}

function CommonPlaceSurface({
  surface,
  onSelectSurface,
}: {
  surface: CommonPlaceSurfaceId;
  onSelectSurface: (surface: CommonPlaceSurfaceId) => void;
}) {
  if (surface === "index") return <IndexSurface onSelectSurface={onSelectSurface} />;
  if (surface === "threads") return <ThreadsSurface />;
  if (surface === "write") return <WriteSurface />;
  if (surface === "artifacts") return <ArtifactsSurface />;
  if (DATA_VIEW_IDS.has(surface)) return <DataLensSurface viewId={surface} onSelectSurface={onSelectSurface} />;
  if (ACCOUNT_SURFACE_IDS.has(surface)) return <SystemSurface surface={surface} />;

  return <IndexSurface onSelectSurface={onSelectSurface} />;
}

function SurfaceFrame({
  eyebrow,
  title,
  description,
  children,
  rail,
}: {
  eyebrow: string;
  title: string;
  description: string;
  children: React.ReactNode;
  rail?: React.ReactNode;
}) {
  return (
    <div className="cpw-ia-scroll">
      <div className="cpw-ia-surface">
        <header className="cpw-ia-header">
          <div>
            <span>{eyebrow}</span>
            <h1>{title}</h1>
            <p>{description}</p>
          </div>
          {rail ? <div className="cpw-ia-header-rail">{rail}</div> : null}
        </header>
        {children}
      </div>
    </div>
  );
}

function IndexSurface({ onSelectSurface }: { onSelectSurface: (surface: CommonPlaceSurfaceId) => void }) {
  const sources = [
    { label: "Emails", count: "0", tone: "blue" },
    { label: "Notes", count: "0", tone: "gold" },
    { label: "Files", count: "4", tone: "teal" },
    { label: "Tasks", count: "1", tone: "orange" },
  ];
  const organized = [
    { title: "Spec bundle", target: "Files / Specs", state: "routed" },
    { title: "Code harness notes", target: "Threads", state: "linked" },
    { title: "Scene package proof", target: "Artifacts", state: "draft" },
  ];

  return (
    <SurfaceFrame
      eyebrow="Thursday, June 25"
      title="Index"
      description="Daily triage, confidence-line decisions, and automatic filing stay here. The name remains Index, not Inbox."
      rail={<DaySegment />}
    >
      <div className="cpw-ia-grid cpw-index-grid">
        <section className="cpw-ia-panel">
          <PanelTitle icon={Inbox} eyebrow="Sources" title="Current intake" />
          <div className="cpw-source-list">
            {sources.map((source) => (
              <button
                key={source.label}
                className="cpw-source-row"
                data-tone={source.tone}
                type="button"
                onClick={() => source.label === "Files" && onSelectSurface("files")}
              >
                <span className="cpw-source-dot" />
                <span>{source.label}</span>
                <small>{source.count}</small>
              </button>
            ))}
          </div>
        </section>
        <section className="cpw-ia-panel cpw-index-decision">
          <PanelTitle icon={Route} eyebrow="Needs you" title="Confidence line" />
          <div className="cpw-empty-dashed">
            <span>All clear. Nothing is waiting on a routing decision.</span>
          </div>
          <div className="cpw-ia-action-row">
            <button type="button" onClick={() => onSelectSurface("threads")}>
              Open work thread
            </button>
            <button type="button" onClick={() => onSelectSurface("code")}>
              Start code agent
            </button>
          </div>
        </section>
        <section className="cpw-ia-panel">
          <PanelTitle icon={CheckCircle2} eyebrow="Organized today" title="Recent routes" />
          <div className="cpw-object-list">
            {organized.map((item) => (
              <article key={item.title} className="cpw-object-row">
                <div>
                  <strong>{item.title}</strong>
                  <span>{item.target}</span>
                </div>
                <small>{item.state}</small>
              </article>
            ))}
          </div>
        </section>
      </div>
    </SurfaceFrame>
  );
}

function ThreadsSurface() {
  const threads = [
    { title: "Coding harness IA", objects: "7 objects", trace: "agent run + diff review", state: "active" },
    { title: "SceneOS renderer contract", objects: "3 artifacts", trace: "OpenUI guardrail", state: "review" },
    { title: "RustyRed data lenses", objects: "12 records", trace: "descriptor registry", state: "stored" },
  ];

  return (
    <SurfaceFrame
      eyebrow="Work record"
      title="Threads"
      description="Persistent agent conversations are tied to the work, objects, artifacts, and run traces they produced."
    >
      <div className="cpw-ia-grid cpw-two-column">
        <section className="cpw-ia-panel">
          <PanelTitle icon={MessageSquareText} eyebrow="Threads" title="Work conversations" />
          <div className="cpw-object-list">
            {threads.map((thread) => (
              <article key={thread.title} className="cpw-thread-row">
                <CircleDot size={13} />
                <div>
                  <strong>{thread.title}</strong>
                  <span>{thread.objects}</span>
                </div>
                <small>{thread.state}</small>
              </article>
            ))}
          </div>
        </section>
        <section className="cpw-ia-panel">
          <PanelTitle icon={Workflow} eyebrow="Context" title="Thread returns to its work" />
          <div className="cpw-thread-context">
            {threads.map((thread) => (
              <div key={thread.trace}>
                <span>{thread.trace}</span>
              </div>
            ))}
          </div>
        </section>
      </div>
    </SurfaceFrame>
  );
}

function WriteSurface() {
  const commands = ["/note", "/task", "/cite", "/deepen", "/scene"];

  return (
    <SurfaceFrame
      eyebrow="Writing"
      title="Write"
      description="Notebooks and notes live here. Compose is the editor state, not a separate page."
    >
      <div className="cpw-ia-grid cpw-write-grid">
        <section className="cpw-ia-panel">
          <PanelTitle icon={BookOpen} eyebrow="Notebooks" title="Writing containers" />
          <div className="cpw-object-list">
            {["Research", "Project notes", "Drafts"].map((notebook) => (
              <article key={notebook} className="cpw-object-row">
                <div>
                  <strong>{notebook}</strong>
                  <span>yrs synced editor scope</span>
                </div>
                <small>open</small>
              </article>
            ))}
          </div>
        </section>
        <section className="cpw-ia-panel cpw-write-editor">
          <PanelTitle icon={PenLine} eyebrow="Compose" title="Block editor destination" />
          <div className="cpw-editor-paper">
            <h2>Untitled note</h2>
            <p>Use the omnibar to attach context, deepen a paragraph, or turn selected notes into a scene package.</p>
            <div className="cpw-slash-row">
              {commands.map((command) => (
                <button key={command} type="button">
                  {command}
                </button>
              ))}
            </div>
          </div>
        </section>
      </div>
    </SurfaceFrame>
  );
}

function DataLensSurface({
  viewId,
  onSelectSurface,
}: {
  viewId: CommonPlaceSurfaceId;
  onSelectSurface: (surface: CommonPlaceSurfaceId) => void;
}) {
  const descriptor = descriptorForView(viewId);
  const Icon = DATA_VIEW_ICONS[viewId] ?? Database;
  const dataViewId = normalizeCommonplaceRustyRedViewId(viewId);
  const data = useCommonplaceRustyRedData(dataViewId);

  return (
    <SurfaceFrame
      eyebrow="RustyRed data"
      title={descriptor.label}
      description="Data views are modular lenses over objects. The active lens declares query shape, renderer family, and emitted actions."
      rail={<LensTabs active={viewId} onSelectSurface={onSelectSurface} />}
    >
      <div className="cpw-ia-grid cpw-data-grid">
        <section className="cpw-ia-panel cpw-data-main">
          <PanelTitle icon={Icon} eyebrow="View" title={`${descriptor.label} lens`} />
          <DataVisualization descriptor={descriptor} payload={data.payload} isLoading={data.isLoading} />
        </section>
        <section className="cpw-ia-panel">
          <PanelTitle icon={Database} eyebrow="ViewDescriptor" title={descriptor.viewDescriptorId} />
          <DescriptorList descriptor={descriptor} />
          <DataContractSummary payload={data.payload} isLoading={data.isLoading} error={data.error} />
        </section>
      </div>
    </SurfaceFrame>
  );
}

function ArtifactsSurface() {
  return (
    <SurfaceFrame
      eyebrow="Generated work"
      title="Artifacts"
      description="Saved outputs and full-canvas interactive scenes relaunch here. SceneOS is the engine; Artifacts is the home."
    >
      <div className="cpw-ia-grid cpw-artifact-grid">
        <section className="cpw-ia-panel">
          <PanelTitle icon={WandSparkles} eyebrow="SceneOS + OpenUI" title="Registered renderers" />
          <div className="cpw-object-list">
            {COMMONPLACE_SCENE_RENDERERS.map((renderer) => (
              <article key={renderer.id} className="cpw-object-row">
                <div>
                  <strong>{renderer.label}</strong>
                  <span>{renderer.capability}</span>
                </div>
                <small>{renderer.status}</small>
              </article>
            ))}
          </div>
        </section>
        <section className="cpw-ia-panel cpw-scene-proof">
          <PanelTitle icon={Sparkles} eyebrow="Artifact" title="Scene package preview" />
          <div className="cpw-scene-card">
            <div className="cpw-scene-map">
              <span />
              <span />
              <span />
            </div>
            <div>
              <strong>CommonPlace coding harness</strong>
              <p>Manifest, datasets, traces, actions, provenance, renderer capabilities, and fallbacks are preserved before save.</p>
              <button type="button">Confirm artifact</button>
            </div>
          </div>
        </section>
      </div>
    </SurfaceFrame>
  );
}

function SystemSurface({ surface }: { surface: CommonPlaceSurfaceId }) {
  const rows: Record<string, readonly { label: string; value: string; icon: SidebarIcon }[]> = {
    account: [
      { label: "Profile", value: "Travis-Gilbert", icon: UserCircle },
      { label: "Billing", value: "workspace account", icon: CreditCard },
    ],
    agents: [
      { label: "Heads", value: "Router, Planner, Editor, Reviewer", icon: Sparkles },
      { label: "ACP", value: "bring your own agent", icon: Link2 },
    ],
    engine: [
      { label: "Substrate", value: "RustyRed ready", icon: Cpu },
      { label: "Instant KG", value: "tenant scoped", icon: Layers3 },
    ],
    desktop: [
      { label: "Desktop app", value: "local engine bridge", icon: MonitorCog },
      { label: "Connectors", value: "outside tools", icon: Upload },
    ],
    settings: [
      { label: "Preferences", value: "app-level behavior", icon: Settings },
      { label: "Command palette", value: "shared quick actions", icon: Command },
    ],
  };

  return (
    <SurfaceFrame
      eyebrow="Configuration"
      title={SURFACE_LABELS[surface]}
      description="Account and system configuration stays below the work surfaces."
    >
      <section className="cpw-ia-panel">
        <div className="cpw-system-grid">
          {(rows[surface] ?? rows.settings).map((row) => {
            const Icon = row.icon;
            return (
              <article key={row.label} className="cpw-system-row">
                <Icon size={18} />
                <div>
                  <strong>{row.label}</strong>
                  <span>{row.value}</span>
                </div>
              </article>
            );
          })}
        </div>
      </section>
    </SurfaceFrame>
  );
}

function CommonPlaceAmbientOmnibar({
  surface,
  onSelectSurface,
}: {
  surface: CommonPlaceSurfaceId;
  onSelectSurface: (surface: CommonPlaceSurfaceId) => void;
}) {
  return (
    <div className="cpw-code-agent-footer cpw-ambient-agent-footer">
      <form
        className={omnibarSurfaceClass("ambient", "cpw-code-agent-omnibar cpw-ambient-omnibar")}
        onSubmit={(event) => event.preventDefault()}
      >
        <textarea
          className="cpw-code-agent-input"
          placeholder={`Ask the Theorem agent about ${SURFACE_LABELS[surface]}`}
          rows={1}
          suppressHydrationWarning
        />
        <div className={omnibarRowClass("cpw-code-agent-omnibar-row")}>
          <div className="cpw-code-agent-tool-icons">
            <button className={omnibarIconButtonClass(false, "cpw-code-agent-tool-button")} type="button" aria-label="Attach context" title="Attach context" onClick={() => onSelectSurface("files")}>
              <Paperclip size={18} />
            </button>
            <button className={omnibarIconButtonClass(false, "cpw-code-agent-tool-button")} type="button" aria-label="Web context" title="Web context">
              <Globe size={18} />
            </button>
            <button className={omnibarIconButtonClass(false, "cpw-code-agent-tool-button")} type="button" aria-label="Instant KG" title="Instant KG" onClick={() => onSelectSurface("graph")}>
              <Layers3 size={18} />
            </button>
            <button className={omnibarIconButtonClass(false, "cpw-code-agent-tool-button")} type="button" aria-label="Git-aware" title="Git-aware" onClick={() => onSelectSurface("code")}>
              <GitBranch size={18} />
            </button>
          </div>
          <button className={omnibarSendButtonClass("cpw-code-agent-send")} type="submit" aria-label="Send message">
            <ArrowUp size={18} />
          </button>
        </div>
      </form>
    </div>
  );
}

function CommandPalettePreview({
  onSelectSurface,
  onClose,
}: {
  onSelectSurface: (surface: CommonPlaceSurfaceId) => void;
  onClose: () => void;
}) {
  const quickActions = COMMONPLACE_TOOLBOX.flatMap((group) => group.items.map((item) => ({ ...item, group: group.label })));
  const destinations = [
    ...COMMONPLACE_WORK_PAGES,
    ...COMMONPLACE_DATA_VIEWS,
    ...COMMONPLACE_ACCOUNT_ITEMS,
  ];

  return (
    <div className="cpw-command-backdrop" role="presentation" onClick={onClose}>
      <div className="cpw-command-palette" role="dialog" aria-label="CommonPlace command palette" onClick={(event) => event.stopPropagation()}>
        <div className="cpw-command-input">
          <Command size={16} />
          <span>Command palette</span>
          <small>Cmd K</small>
        </div>
        <div className="cpw-command-columns">
          <div>
            <span className="cpw-command-section">Go</span>
            {destinations.map((item) => (
              <button
                key={item.id}
                type="button"
                onClick={() => {
                  onSelectSurface(item.id as CommonPlaceSurfaceId);
                  onClose();
                }}
              >
                <span>{item.label}</span>
                <small>{item.placement}</small>
              </button>
            ))}
          </div>
          <div>
            <span className="cpw-command-section">Quick actions</span>
            {quickActions.map((item) => {
              const Icon = QUICK_ACTION_ICONS[item.id] ?? Plus;
              return (
                <button key={`${item.group}-${item.id}`} type="button" onClick={onClose}>
                  <Icon size={14} />
                  <span>{item.label}</span>
                  <small>{item.group}</small>
                </button>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}

function PanelTitle({ icon: Icon, eyebrow, title }: { icon: SidebarIcon; eyebrow: string; title: string }) {
  return (
    <div className="cpw-panel-title">
      <Icon size={16} />
      <div>
        <span>{eyebrow}</span>
        <strong>{title}</strong>
      </div>
    </div>
  );
}

function DaySegment() {
  return (
    <div className="cpw-segmented cpw-ia-segmented">
      <button type="button" data-active="true">
        Day
      </button>
      <button type="button">Week</button>
      <button type="button">Month</button>
    </div>
  );
}

function LensTabs({
  active,
  onSelectSurface,
}: {
  active: CommonPlaceSurfaceId;
  onSelectSurface: (surface: CommonPlaceSurfaceId) => void;
}) {
  return (
    <div className="cpw-lens-tabs" aria-label="Data lenses">
      {COMMONPLACE_DATA_VIEWS.map((view) => {
        const Icon = DATA_VIEW_ICONS[view.id] ?? Database;
        return (
          <button
            key={view.id}
            type="button"
            data-active={active === view.id}
            onClick={() => onSelectSurface(view.id as CommonPlaceSurfaceId)}
          >
            <Icon size={14} />
            <span>{view.label}</span>
          </button>
        );
      })}
    </div>
  );
}

function DescriptorList({ descriptor }: { descriptor: CommonplaceDataViewDescriptor }) {
  const rows = [
    ["Status", descriptor.status ?? "enabled"],
    ["Objects", descriptor.objectTypes.join(", ")],
    ["Renderers", descriptor.renderers.join(", ")],
    ["Actions", descriptor.actions.join(", ")],
    ["Query", descriptor.query.types.join(", ")],
    ["Rank", descriptor.query.rank?.join(", ") ?? "none"],
    ["Slice", descriptor.query.slice?.join(", ") ?? "none"],
    ...(descriptor.deferredReason ? [["Deferred", descriptor.deferredReason]] : []),
  ];

  return (
    <dl className="cpw-descriptor-list">
      {rows.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function DataContractSummary({
  payload,
  isLoading,
  error,
}: {
  payload: CommonplaceRustyRedDataPayload;
  isLoading: boolean;
  error?: string;
}) {
  const rows = [
    ["Contract", payload.version],
    ["Source", isLoading ? "loading" : payload.source.mode],
    ["Endpoint", payload.source.endpoint ?? "not configured"],
    ["Objects", String(payload.objectSet.objects.length)],
    ["Graph links", String(payload.graph.links.length)],
    ["NocoBase", `${payload.nocobase.packageName} ${payload.nocobase.status}`],
    ["Deck", payload.geo.note],
    ...(payload.source.message ? [["Message", payload.source.message]] : []),
    ...(error ? [["Client", error]] : []),
  ];

  return (
    <dl className="cpw-descriptor-list cpw-data-contract-list">
      {rows.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function DataVisualization({
  descriptor,
  payload,
  isLoading,
}: {
  descriptor: CommonplaceDataViewDescriptor;
  payload: CommonplaceRustyRedDataPayload;
  isLoading: boolean;
}) {
  if (descriptor.status === "deferred") return <MapLens payload={payload} />;
  if (descriptor.id === "graph") return <GraphLens payload={payload} isLoading={isLoading} />;
  if (descriptor.id === "table") return <TableLens payload={payload} />;
  if (descriptor.id === "timeline") return <TimelineLens payload={payload} />;
  if (descriptor.id === "clips") return <ClipsLens payload={payload} />;
  return <FilesLens payload={payload} />;
}

function FilesLens({ payload }: { payload: CommonplaceRustyRedDataPayload }) {
  const files = payload.items.length
    ? payload.items.map((item) => ({ id: item.id, path: item.path ?? undefined, title: item.title }))
    : payload.objectSet.objects
        .filter((object) => object.type === "file")
        .map((object) => ({
          id: object.id,
          path: typeof object.properties.path === "string" ? object.properties.path : undefined,
          title: String(object.properties.title ?? object.id),
        }));

  return (
    <div className="cpw-files-lens">
      {files.map((item) => {
        const label = item.path || item.title;
        return (
          <div key={item.id} className="cpw-file-row" data-kind={item.path ? "file" : "directory"}>
            {item.path ? <FileText size={13} /> : <Folder size={13} />}
            <span>{label}</span>
          </div>
        );
      })}
      {!files.length && (
        <div className="cpw-file-row" data-kind="directory">
          <FileText size={13} />
          <span>No RustyRed file objects matched this view.</span>
        </div>
      )}
    </div>
  );
}

function GraphLens({ payload, isLoading }: { payload: CommonplaceRustyRedDataPayload; isLoading: boolean }) {
  const nodes = payload.graph.nodes.map((node) => ({
    id: node.id,
    x: node.x,
    y: node.y,
    color: node.color,
    size: node.size,
    label: node.label,
    meta: node.meta,
  }));
  const links = payload.graph.links.map((link) => ({ source: link.source, target: link.target }));

  return (
    <div className="cpw-graph-lens">
      <div className="cpw-history-actions">
        {["snapshot", "diff", "branch", "restore"].map((action) => (
          <button key={action} type="button">
            <History size={13} />
            {action}
          </button>
        ))}
      </div>
      <div className="cpw-graph-map" aria-label="Graph preview">
        {nodes.length ? (
          <>
            <CosmosGraph className="cpw-cosmos-graph" nodes={nodes} links={links} />
            <div className="cpw-graph-legend">
              <strong>{isLoading ? "Loading RustyRed graph" : `${nodes.length} objects`}</strong>
              <span>{links.length} relations via collections, discovery, and briefing</span>
            </div>
          </>
        ) : (
          <div className="cpw-data-empty">No RustyRed graph objects matched this view.</div>
        )}
      </div>
    </div>
  );
}

function TableLens({ payload }: { payload: CommonplaceRustyRedDataPayload }) {
  return (
    <div className="cpw-table-lens">
      <table className="cpw-object-table">
        <thead>
          <tr>
            <th>Object</th>
            <th>Type</th>
            <th>Status</th>
            <th>Source</th>
            <th>Updated</th>
          </tr>
        </thead>
        <tbody>
          {payload.table.rows.map((row) => (
            <tr key={row.id}>
              <td>{row.title}</td>
              <td>{row.type}</td>
              <td>{row.status}</td>
              <td>{row.source}</td>
              <td>{row.updatedAt}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="cpw-nocobase-bridge">
        <strong>{payload.nocobase.packageName}</strong>
        <span>{payload.nocobase.mode} over {payload.nocobase.dataSource}</span>
      </div>
    </div>
  );
}

function MapLens({ payload }: { payload: CommonplaceRustyRedDataPayload }) {
  return (
    <div className="cpw-map-lens cpw-deferred-lens" aria-label="Map preview">
      <strong>Map is deferred.</strong>
      <span>{payload.geo.note}</span>
      <small>{payload.geo.points.length} coordinate objects are available for the future Deck.gl layer.</small>
    </div>
  );
}

function TimelineLens({ payload }: { payload: CommonplaceRustyRedDataPayload }) {
  const events = payload.items.length
    ? payload.items.map((event) => ({
      id: event.id,
      title: event.title,
      updatedAt: new Date(event.updatedAtMs).toLocaleDateString(),
    }))
    : payload.table.rows.map((event) => ({
      id: event.id,
      title: event.title,
      updatedAt: event.updatedAt,
    }));

  return (
    <ol className="cpw-timeline-lens">
      {events.map((event) => (
        <li key={event.id}>
          <Clock3 size={13} />
          <span>{event.title}</span>
          <small>{event.updatedAt}</small>
        </li>
      ))}
    </ol>
  );
}

function ClipsLens({ payload }: { payload: CommonplaceRustyRedDataPayload }) {
  return (
    <div className="cpw-object-list">
      {payload.items.map((clip) => (
        <article key={clip.id} className="cpw-object-row">
          <div>
            <strong>{clip.title}</strong>
            <span>{clip.source ?? clip.path ?? "clipped source with provenance"}</span>
          </div>
          <small>{clip.kind}</small>
        </article>
      ))}
      {!payload.items.length && (
        <article className="cpw-object-row">
          <div>
            <strong>No RustyRed clips matched this view.</strong>
            <span>Clip-like objects will appear here once the CommonPlace GraphQL edge has them.</span>
          </div>
          <small>clips</small>
        </article>
      )}
    </div>
  );
}

function descriptorForView(viewId: CommonPlaceSurfaceId): CommonplaceDataViewDescriptor {
  return COMMONPLACE_DATA_VIEW_DESCRIPTORS.find((descriptor) => descriptor.id === viewId) ?? COMMONPLACE_DATA_VIEW_DESCRIPTORS[0];
}

function CodeAgentRuntime() {
  const [transport, setTransport] = React.useState<CodeAgentTransportId>("api");
  const [messages, setMessages] = React.useState<readonly ThreadMessage[]>(() => [
    assistantMessage({
      id: INITIAL_ASSISTANT_ID,
      text: "Ready.",
      createdAt: INITIAL_DATE,
    }),
  ]);
  const [isRunning, setIsRunning] = React.useState(false);
  const [modelStatuses, setModelStatuses] =
    React.useState<readonly CodeAgentModelStatus[]>(DEFAULT_MODEL_STATUSES);
  const [diffsByMessageId, setDiffsByMessageId] = React.useState<Record<string, readonly CodeDiffArtifact[]>>({});

  const runtime = useExternalStoreRuntime<ThreadMessage>({
    messages,
    isRunning,
    setMessages,
    onNew: async (message) => {
      const prompt = textFromAppendMessage(message);
      const userId = `code-agent:user:${Date.now()}`;
      const assistantId = `code-agent:assistant:${Date.now()}`;

      setMessages((current) => [...current, userMessageFromAppend(message, userId)]);
      setIsRunning(true);
      setModelStatuses(statusesWith({ router: "routing", planner: "queued", editor: "idle", reviewer: "idle" }));

      try {
        const result = await runCodeAgentTurn({
          prompt,
          transport,
          onProgress: (progress) => {
            if (progress.states) setModelStatuses(statusesWith(progress.states));
          },
        });
        if (result.diffs.length > 0) {
          setDiffsByMessageId((current) => ({ ...current, [assistantId]: result.diffs }));
        }
        setMessages((current) => [
          ...current,
          assistantMessage({
            id: assistantId,
            text: result.text,
            createdAt: new Date(),
          }),
        ]);
        setModelStatuses(statusesWith({ router: "success", planner: "success", editor: "success", reviewer: "success" }));
      } catch (error) {
        setMessages((current) => [
          ...current,
          assistantMessage({
            id: assistantId,
            text: `The agent run failed: ${error instanceof Error ? error.message : String(error)}`,
            createdAt: new Date(),
          }),
        ]);
        setModelStatuses(statusesWith({ router: "error", planner: "error", editor: "idle", reviewer: "error" }));
      } finally {
        setIsRunning(false);
      }
    },
    onEdit: async (message) => {
      if (!message.sourceId) return;
      setMessages((current) =>
        current.map((item) => (item.id === message.sourceId ? userMessageFromAppend(message, message.sourceId) : item)),
      );
    },
    onReload: async () => {
      const assistantId = `code-agent:assistant:reload:${Date.now()}`;
      setIsRunning(true);
      setModelStatuses(statusesWith({ router: "routing", planner: "thinking", editor: "idle", reviewer: "idle" }));
      await sleep(360);
      setMessages((current) => [
        ...current,
        assistantMessage({
          id: assistantId,
          text: "Rechecked the last turn.",
          createdAt: new Date(),
        }),
      ]);
      setModelStatuses(statusesWith({ router: "success", planner: "success", editor: "idle", reviewer: "success" }));
      setIsRunning(false);
    },
    onCancel: async () => {
      setModelStatuses(statusesWith({ router: "stopped", planner: "stopped", editor: "stopped", reviewer: "stopped" }));
      setIsRunning(false);
    },
  });

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <ThreadPrimitive.Root className="cpw-code-agent-root">
        <ModelStatusRail statuses={modelStatuses} />
        <ThreadPrimitive.Viewport className="cpw-code-agent-thread" autoScroll>
          <ThreadPrimitive.Empty>
            <div className="cpw-code-agent-empty">Ask the Theorem agent</div>
          </ThreadPrimitive.Empty>
          <ThreadPrimitive.Messages>
            {({ message }) => (
              <CodeAgentMessage
                key={message.id}
                message={message}
                diffs={diffsByMessageId[message.id] ?? []}
              />
            )}
          </ThreadPrimitive.Messages>
        </ThreadPrimitive.Viewport>
        <div className="cpw-code-agent-footer">
          <CommonPlaceAgentComposer
            transport={transport}
            onTransportChange={setTransport}
            isRunning={isRunning}
          />
        </div>
      </ThreadPrimitive.Root>
    </AssistantRuntimeProvider>
  );
}

function ModelStatusRail({ statuses }: { statuses: readonly CodeAgentModelStatus[] }) {
  return (
    <div className="cpw-code-agent-models" aria-label="API model states">
      {statuses.map((model) => {
        const dotState = dotStateForApiModelState(model.state);
        return (
          <div key={model.id} className="cpw-code-agent-model" data-state={model.state}>
            <DotMatrix state={dotState} label={`${model.label} ${model.state}`} />
            <span>{model.label}</span>
            <small>{model.state.replaceAll("_", " ")}</small>
          </div>
        );
      })}
    </div>
  );
}

function CodeAgentMessage({
  message,
  diffs,
}: {
  message: MessageState;
  diffs: readonly CodeDiffArtifact[];
}) {
  if (message.role === "system") return null;

  return (
    <MessagePrimitive.Root className="cpw-code-agent-message" data-role={message.role}>
      <div className="cpw-code-agent-bubble">
        <MessagePrimitive.Parts
          components={{
            Text: message.role === "user" ? UserTextPart : AssistantTextPart,
          }}
        />
        {message.role === "assistant" ? <AssistantDiffStack diffs={diffs} /> : null}
      </div>
    </MessagePrimitive.Root>
  );
}

function UserTextPart() {
  return <MessagePartPrimitive.Text component="p" className="cpw-code-agent-text" smooth={false} />;
}

function AssistantTextPart() {
  return <MessagePartPrimitive.Text component="p" className="cpw-code-agent-text" />;
}

function AssistantDiffStack({ diffs }: { diffs: readonly CodeDiffArtifact[] }) {
  if (diffs.length === 0) return null;

  return (
    <div className="cpw-code-agent-diffs">
      {diffs.map((diff) => (
        <CollapsedDiffArtifact key={diff.id} diff={diff} />
      ))}
    </div>
  );
}

function CollapsedDiffArtifact({ diff }: { diff: CodeDiffArtifact }) {
  const [open, setOpen] = React.useState(false);

  return (
    <details
      className="cpw-code-agent-diff"
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>
        <span>{diff.title}</span>
        <small>
          {diff.path} +{diff.additions} -{diff.deletions}
        </small>
      </summary>
      {open ? (
        <div className="cpw-code-agent-diff-viewer">
          <CodeMirrorMerge
            theme={commonplaceCodeMirrorTheme}
            orientation="a-b"
            highlightChanges
            gutter
            collapseUnchanged={{ margin: 2, minSize: 4 }}
            className="cpw-merge-view"
          >
            <MergeOriginal
              value={diff.before}
              extensions={[...commonplaceCodeExtensions("typescript"), ...readOnlyExtensions]}
            />
            <MergeModified
              value={diff.after}
              extensions={[...commonplaceCodeExtensions("typescript"), ...readOnlyExtensions]}
            />
          </CodeMirrorMerge>
        </div>
      ) : null}
    </details>
  );
}

function CommonPlaceAgentComposer({
  transport,
  onTransportChange,
  isRunning,
}: {
  transport: CodeAgentTransportId;
  onTransportChange: (transport: CodeAgentTransportId) => void;
  isRunning: boolean;
}) {
  return (
    <ComposerPrimitive.Root className={omnibarSurfaceClass("ambient", "cpw-code-agent-omnibar")}>
      <ComposerPrimitive.Input
        className="cpw-code-agent-input"
        placeholder="Ask the Theorem agent"
        submitMode="enter"
        minRows={1}
        maxRows={5}
        suppressHydrationWarning
      />
      <div className={omnibarRowClass("cpw-code-agent-omnibar-row")}>
        <div className="cpw-code-agent-tool-icons">
          <button className={omnibarIconButtonClass(false, "cpw-code-agent-tool-button")} type="button" aria-label="Attach context" title="Attach context">
            <Paperclip size={21} />
          </button>
          <button className={omnibarIconButtonClass(false, "cpw-code-agent-tool-button")} type="button" aria-label="Web context" title="Web context">
            <Globe size={21} />
          </button>
          <AgentTransportMenu transport={transport} onTransportChange={onTransportChange} />
          <button className={omnibarIconButtonClass(false, "cpw-code-agent-tool-button")} type="button" aria-label="Graph context" title="Graph context">
            <GitBranch size={21} />
          </button>
        </div>
        <ComposerPrimitive.Send
          className={omnibarSendButtonClass("cpw-code-agent-send")}
          aria-label={isRunning ? "Agent is running" : "Send message"}
        >
          <ArrowUp size={20} />
        </ComposerPrimitive.Send>
      </div>
    </ComposerPrimitive.Root>
  );
}

function AgentTransportMenu({
  transport,
  onTransportChange,
}: {
  transport: CodeAgentTransportId;
  onTransportChange: (transport: CodeAgentTransportId) => void;
}) {
  const label = transportLabel(transport);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          className={omnibarIconButtonClass(transport !== "api", "cpw-code-agent-tool-button")}
          type="button"
          aria-label={`Agent transport: ${label}`}
          title={`Agent transport: ${label}`}
        >
          <Sparkles size={21} />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        {CODE_AGENT_TRANSPORTS.map((item) => (
          <DropdownMenuItem
            key={item.id}
            onSelect={() => onTransportChange(item.id)}
            className={item.id === transport ? "text-ox" : undefined}
          >
            <span className="font-mono text-[11px]">{item.label}</span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function MersenneBinaryField() {
  const glyphs = React.useMemo(() => createMersenneBinaryGlyphs(), []);

  return (
    <div className="cpw-mersenne-field" aria-hidden>
      {glyphs.map((glyph) => {
        const style = {
          "--mt-row": glyph.row,
          "--mt-column": glyph.column,
          "--mt-opacity": glyph.opacity,
          "--mt-delay": `${glyph.delay}ms`,
        } as React.CSSProperties;
        return (
          <span key={glyph.id} style={style}>
            {glyph.value}
          </span>
        );
      })}
    </div>
  );
}

function userMessageFromAppend(message: AppendMessage, id: string): ThreadMessage {
  return {
    id,
    role: "user",
    createdAt: new Date(),
    content: message.content as readonly ThreadUserMessagePart[],
    attachments: message.attachments ?? [],
    metadata: { custom: {} },
  };
}

function assistantMessage({
  id,
  text,
  createdAt,
}: {
  id: string;
  text: string;
  createdAt: Date;
}): ThreadMessage {
  const content: readonly ThreadAssistantMessagePart[] = [{ type: "text", text }];

  return {
    id,
    role: "assistant",
    createdAt,
    content,
    status: { type: "complete", reason: "stop" },
    metadata: {
      unstable_state: null,
      unstable_annotations: [],
      unstable_data: [],
      steps: [],
      custom: {},
    },
  };
}

function textFromAppendMessage(message: AppendMessage) {
  return message.content
    .map((part) => (part.type === "text" ? part.text : ""))
    .join("\n")
    .trim();
}

function statusesWith(states: Partial<Record<CodeAgentModelStatus["id"], ApiModelState>>) {
  return DEFAULT_MODEL_STATUSES.map((model) => ({
    ...model,
    state: states[model.id] ?? model.state,
  }));
}

function transportLabel(transport: CodeAgentTransportId) {
  return CODE_AGENT_TRANSPORTS.find((item) => item.id === transport)?.label ?? transport;
}

function sleep(ms: number) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}
