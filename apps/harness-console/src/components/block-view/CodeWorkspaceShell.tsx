"use client";

import * as React from "react";
import {
  ArrowUp,
  Boxes,
  Bell,
  ChevronLeft,
  Clock3,
  Code2,
  Command,
  Cpu,
  Database,
  Folder,
  GitBranch,
  Globe,
  Gauge,
  Inbox,
  Layers3,
  Map,
  MessageSquareText,
  MonitorCog,
  Paperclip,
  PenLine,
  Plus,
  Search,
  SlidersHorizontal,
  Settings,
  Scissors,
  Sparkles,
  SquareTerminal,
  Table2,
  UserCircle,
  WandSparkles,
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
  COMMONPLACE_DATA_VIEWS,
  COMMONPLACE_OMNIBAR_CAPABILITIES,
  COMMONPLACE_TOOLBOX,
  COMMONPLACE_WORK_PAGES,
  type CommonplaceIaItem,
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
import { createMersenneBinaryGlyphs } from "@/lib/commonplace/mersenne-pattern";
import {
  commonplaceCodeExtensions,
  commonplaceCodeMirrorTheme,
  readOnlyExtensions,
} from "./commonplace-code-theme";

const MergeOriginal = CodeMirrorMerge.Original;
const MergeModified = CodeMirrorMerge.Modified;
type SidebarIcon = React.ComponentType<{ size?: number; className?: string }>;

const INITIAL_ASSISTANT_ID = "code-agent:assistant:intro";
const INITIAL_DATE = new Date("2026-06-25T00:00:00.000Z");

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

export function CodeWorkspaceShell() {
  return (
    <main className="commonplace-workspace-theme cpw-shell" data-merge-scope="preview-route">
      <CommonPlacePreviewSidebar />
      <CodeWorkspaceStage />
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

// Preview-only IA fixture for this Theorem test route.
// Do not move this component into the production CommonPlace app shell; only the
// plain IA data in `lib/commonplace/information-architecture.ts` is intended to
// survive the real merge.
function CommonPlacePreviewSidebar() {
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

      <OmnibarPreview />

      <nav className="cpw-nav" aria-label="CommonPlace sections">
        <NavSection label="Work" />
        {COMMONPLACE_WORK_PAGES.map((item) => (
          <NavItem key={item.id} item={item} icon={WORK_ICONS[item.id] ?? Folder} active={item.id === "code"} />
        ))}

        <NavSection label="Data" />
        {COMMONPLACE_DATA_VIEWS.map((item) => (
          <NavItem key={item.id} item={item} icon={DATA_VIEW_ICONS[item.id] ?? Database} />
        ))}

        <ToolboxPreview groups={COMMONPLACE_TOOLBOX} />

        <NavSection label="Account" />
        {COMMONPLACE_ACCOUNT_ITEMS.map((item) => (
          <NavItem key={item.id} item={item} icon={ACCOUNT_ICONS[item.id] ?? SlidersHorizontal} />
        ))}
      </nav>

      <button className="cpw-engine-status" type="button">
        <span className="cpw-engine-dot" />
        <span>Engine</span>
      </button>
    </aside>
  );
}

function OmnibarPreview() {
  return (
    <section className="cpw-omnibar-preview" aria-label="Agent omnibar">
      <button className="cpw-search" type="button" aria-label="Ask the Theorem agent">
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

function NavItem({ item, icon: Icon, active }: { item: CommonplaceIaItem; icon: SidebarIcon; active?: boolean }) {
  return (
    <button
      className="cpw-nav-item"
      data-active={active ? "true" : "false"}
      data-placement={item.placement}
      type="button"
      title={item.description}
      aria-current={active ? "page" : undefined}
    >
      <Icon size={17} />
      <span>{item.label}</span>
      {active ? <span className="cpw-nav-marker" /> : null}
      {item.count ? <span className="cpw-nav-count">{item.count}</span> : null}
    </button>
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
