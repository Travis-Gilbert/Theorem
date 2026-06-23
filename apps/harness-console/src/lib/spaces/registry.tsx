"use client";

import {
  Bot,
  Brain,
  Boxes,
  Code2,
  Compass,
  FileText,
  Gauge,
  Home,
  KeyRound,
  LayoutGrid,
  Network,
  Orbit,
  Paintbrush,
  Plug,
  Radio,
  Settings,
  Sparkles,
} from "lucide-react";
import type { SpaceTypeDefinition } from "./types";

function EmptySpaceView() {
  return <div className="min-h-0" />;
}

function EmptySpaceEditor() {
  return <div className="min-h-0" />;
}

function CodeMirrorEditorSlot() {
  return <div data-editor="velt-codemirror" className="min-h-0" />;
}

function TipTapEditorSlot() {
  return <div data-editor="tiptap" className="min-h-0" />;
}

function BlockSuiteEditorSlot() {
  return <div data-editor="blocksuite" className="min-h-0" />;
}

const DEFINITIONS: SpaceTypeDefinition[] = [
  {
    typeKey: "home",
    defaultLabel: "Home",
    icon: Home,
    href: "/",
    capabilities: ["view"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "auto-organizer",
    defaultLabel: "Auto-Organizer",
    icon: Sparkles,
    href: "/inbox",
    capabilities: ["view", "organize"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "browser",
    defaultLabel: "Browser",
    icon: Compass,
    href: "/connections",
    capabilities: ["view"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "code",
    defaultLabel: "Code",
    icon: Code2,
    href: "/skills",
    capabilities: ["view", "editor"],
    view: EmptySpaceView,
    defaultEditor: CodeMirrorEditorSlot,
  },
  {
    typeKey: "notes",
    defaultLabel: "Notes",
    icon: FileText,
    href: "/memory",
    capabilities: ["view", "editor"],
    view: EmptySpaceView,
    defaultEditor: TipTapEditorSlot,
  },
  {
    typeKey: "canvas",
    defaultLabel: "Canvas",
    icon: LayoutGrid,
    href: "/canvas",
    capabilities: ["view", "editor"],
    view: EmptySpaceView,
    defaultEditor: BlockSuiteEditorSlot,
  },
  {
    typeKey: "accounts",
    defaultLabel: "Accounts",
    icon: Orbit,
    capabilities: ["account"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "agents",
    defaultLabel: "Agents",
    icon: Bot,
    href: "/agent",
    capabilities: ["view", "agent"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "agent-thread",
    defaultLabel: "Agent",
    icon: Bot,
    href: "/agent",
    capabilities: ["view", "agent"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "memory",
    defaultLabel: "Memory",
    icon: Brain,
    href: "/memory",
    capabilities: ["view", "agent"],
    view: EmptySpaceView,
    defaultEditor: TipTapEditorSlot,
  },
  {
    typeKey: "skills",
    defaultLabel: "Skills",
    icon: Boxes,
    href: "/skills",
    capabilities: ["view", "agent", "editor"],
    view: EmptySpaceView,
    defaultEditor: CodeMirrorEditorSlot,
  },
  {
    typeKey: "rooms",
    defaultLabel: "Rooms",
    icon: Radio,
    href: "/rooms",
    capabilities: ["view", "agent"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "runs",
    defaultLabel: "Runs",
    icon: Network,
    href: "/runs",
    capabilities: ["view", "agent"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "mcp-hub",
    defaultLabel: "MCP Hub",
    icon: Plug,
    href: "/connections",
    capabilities: ["view", "account"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "providers",
    defaultLabel: "Providers",
    icon: Plug,
    href: "/providers",
    capabilities: ["view", "account"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "connections",
    defaultLabel: "Connections",
    icon: Network,
    href: "/connections",
    capabilities: ["view", "account"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "api-keys",
    defaultLabel: "API Keys",
    icon: KeyRound,
    href: "/keys",
    capabilities: ["view", "account"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "usage",
    defaultLabel: "Usage",
    icon: Gauge,
    href: "/usage",
    capabilities: ["view", "account"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "settings",
    defaultLabel: "Settings",
    icon: Settings,
    href: "/settings",
    capabilities: ["view", "account"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
  {
    typeKey: "generic-collection",
    defaultLabel: "Collection",
    icon: Paintbrush,
    capabilities: ["view", "editor", "plugin"],
    view: EmptySpaceView,
    defaultEditor: EmptySpaceEditor,
  },
];

export const spaceTypeDefinitions = new Map(
  DEFINITIONS.map((definition) => [definition.typeKey, definition]),
);

export function getSpaceTypeDefinition(typeKey: string): SpaceTypeDefinition | undefined {
  return spaceTypeDefinitions.get(typeKey);
}

export function registerSpaceTypeDefinition(definition: SpaceTypeDefinition): void {
  spaceTypeDefinitions.set(definition.typeKey, definition);
}

export function listSpaceTypeDefinitions(): SpaceTypeDefinition[] {
  return [...spaceTypeDefinitions.values()];
}
