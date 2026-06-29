"use client";

import { useState } from "react";
import { cn } from "@/lib/utils";
import {
  Folder,
  FolderOpen,
  File as FileIcon,
  ChevronRight,
  Trash2,
  Pencil,
  KeyRound,
  type LucideIcon,
} from "lucide-react";

/** The DocTree — RustyRed's graph-native document tree (its leaves ARE documents,
 *  FUSE-projectable as a real filesystem). So it reads as a file tree: every leaf
 *  is a file with a real extension, drawn with its vibrant a-file-icon glyph
 *  (/public/file-icons). Folders use a neutral folder glyph; api.key keeps the
 *  oxblood key as the one action item. Expand/collapse, select, hover rename/delete. */

/** dataTransfer mime for dragging a doc leaf out to the UploadDock (cosmetic). */
export const DOC_DND_MIME = "application/x-theorem-doc";

type Node = {
  id: string;
  name: string;
  type: "doc" | "folder";
  children?: Node[];
  icon?: LucideIcon;
  fileIcon?: string; // a-file-icon name (/public/file-icons) for real file leaves
  accent?: boolean;
};

// Reframed as a real file tree: every leaf is a file with an extension, so the
// vibrant a-file-icon file-type glyphs fit. The memory lifecycle (warm working
// set -> consolidated -> ColdTier) now reads from the filenames.
const INITIAL: Node[] = [
  { id: "docs", name: "README.md", type: "doc", fileIcon: "markdown" },
  {
    id: "memory",
    name: "memory",
    type: "folder",
    children: [
      { id: "mem-memories", name: "memories.md", type: "doc", fileIcon: "markdown" },
      { id: "mem-collections", name: "collections.json", type: "doc", fileIcon: "json" },
      { id: "mem-tags", name: "tags.yaml", type: "doc", fileIcon: "yaml" },
    ],
  },
  {
    id: "agent-memory",
    name: "agent-memory",
    type: "folder",
    children: [
      { id: "am-working", name: "working.md", type: "doc", fileIcon: "markdown" },
      { id: "am-short", name: "short-term.json", type: "doc", fileIcon: "json" },
      { id: "am-long", name: "long-term.json", type: "doc", fileIcon: "json" },
      { id: "am-cold", name: "cold-storage", type: "doc", fileIcon: "snowflake" },
      { id: "am-post", name: "postmortems.md", type: "doc", fileIcon: "markdown" },
      { id: "am-dec", name: "decisions.md", type: "doc", fileIcon: "markdown" },
    ],
  },
  {
    id: "projects",
    name: "projects",
    type: "folder",
    children: [
      {
        id: "p-theorem",
        name: "theorem",
        type: "folder",
        children: [{ id: "p-theorem-spec", name: "spec.md", type: "doc", fileIcon: "markdown" }],
      },
      {
        id: "p-cp",
        name: "commonplace",
        type: "folder",
        children: [{ id: "p-cp-notes", name: "notes.md", type: "doc", fileIcon: "markdown" }],
      },
    ],
  },
  { id: "api-key", name: "api.key", type: "doc", icon: KeyRound, accent: true },
];

function removeNode(nodes: Node[], id: string): Node[] {
  return nodes
    .filter((n) => n.id !== id)
    .map((n) => (n.children ? { ...n, children: removeNode(n.children, id) } : n));
}

export function DocTree() {
  const [tree, setTree] = useState<Node[]>(INITIAL);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({ memory: true, "agent-memory": true });
  const [selected, setSelected] = useState<string | null>("mem-memories");

  const toggle = (id: string) => setExpanded((s) => ({ ...s, [id]: !s[id] }));

  const render = (nodes: Node[], level = 0) =>
    nodes.map((n) => {
      const isOpen = !!expanded[n.id];
      const isFolder = n.type === "folder";
      const Leaf = n.icon ?? FileIcon;
      return (
        <div key={n.id} className="group/node">
          <div
            role="treeitem"
            aria-expanded={isFolder ? isOpen : undefined}
            aria-selected={selected === n.id}
            tabIndex={0}
            draggable={!isFolder}
            onDragStart={
              isFolder
                ? undefined
                : (e) => {
                    e.dataTransfer.setData(
                      DOC_DND_MIME,
                      JSON.stringify({
                        name: n.name,
                        type: n.name.endsWith(".md") ? "text/markdown" : "text/plain",
                        size: n.name.length * 137 + 2048, // cosmetic, deterministic
                      }),
                    );
                    e.dataTransfer.effectAllowed = "copy";
                  }
            }
            onClick={() => {
              setSelected(n.id);
              if (isFolder) toggle(n.id);
            }}
            className={cn(
              "group flex items-center gap-2 rounded-md py-1.5 pr-1.5 text-[14px] transition-colors",
              isFolder ? "cursor-pointer" : "cursor-grab active:cursor-grabbing",
              selected === n.id ? "bg-black/[.06] text-ink" : "text-muted-foreground hover:bg-black/[.04] hover:text-ink",
            )}
            style={{ paddingLeft: level * 14 + 6 }}
          >
            {isFolder ? (
              <ChevronRight
                size={14}
                className={cn("flex-none text-muted-foreground transition-transform", isOpen && "rotate-90")}
              />
            ) : (
              <span className="w-3.5 flex-none" />
            )}
            {isFolder ? (
              isOpen ? (
                <FolderOpen size={16} className="flex-none" style={{ color: "var(--ox)" }} />
              ) : (
                <Folder size={16} className="flex-none text-muted-foreground" />
              )
            ) : n.fileIcon ? (
              // eslint-disable-next-line @next/next/no-img-element
              <img src={`/file-icons/${n.fileIcon}.svg`} alt="" className="h-[15px] w-[15px] flex-none" />
            ) : (
              <Leaf size={15} className="flex-none" style={{ color: n.accent ? "var(--ox)" : undefined }} />
            )}
            <span className="flex-1 truncate">{n.name}</span>

            {/* hover actions */}
            <span className="flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
              <button
                type="button"
                aria-label={`Rename ${n.name}`}
                onClick={(e) => e.stopPropagation()}
                className="grid h-6 w-6 place-items-center rounded text-muted-foreground hover:text-ink"
              >
                <Pencil size={13} />
              </button>
              <button
                type="button"
                aria-label={`Delete ${n.name}`}
                onClick={(e) => {
                  e.stopPropagation();
                  setTree((t) => removeNode(t, n.id));
                }}
                className="grid h-6 w-6 place-items-center rounded text-muted-foreground hover:text-ink"
              >
                <Trash2 size={13} />
              </button>
            </span>
          </div>

          {isFolder && n.children && isOpen && <div role="group">{render(n.children, level + 1)}</div>}
        </div>
      );
    });

  return (
    <div role="tree" className="flex flex-col gap-0.5">
      {render(tree)}
    </div>
  );
}
