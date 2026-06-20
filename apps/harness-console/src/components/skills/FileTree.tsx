"use client";

import * as React from "react";
import {
  ChevronRight,
  File as FileIcon,
  FileCode,
  FileText,
  FolderOpen,
  Plus,
  Trash2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { SkillFile } from "@/lib/harness";

/**
 * The skill pack's file tree, modeled on Mistral's Skills screen: SKILL.md sits
 * at the root, additional files and folders nest below. Selecting a file raises
 * it into the editor. Folders are derived from the slash-separated paths, not a
 * separate model, so the tree always reflects the real file set.
 */

interface TreeFolder {
  name: string;
  path: string; // folder prefix, e.g. "checklists"
  folders: TreeFolder[];
  files: SkillFile[];
}

function iconFor(file: SkillFile) {
  switch (file.language) {
    case "markdown":
    case "text":
      return FileText;
    case "python":
    case "typescript":
    case "rust":
      return FileCode;
    default:
      return FileIcon;
  }
}

/** Build a nested folder tree from flat slash-separated paths. */
function buildTree(files: SkillFile[]): TreeFolder {
  const root: TreeFolder = { name: "", path: "", folders: [], files: [] };
  const ensure = (parent: TreeFolder, name: string, path: string): TreeFolder => {
    let child = parent.folders.find((f) => f.name === name);
    if (!child) {
      child = { name, path, folders: [], files: [] };
      parent.folders.push(child);
    }
    return child;
  };
  for (const file of files) {
    const parts = file.path.split("/");
    const fileName = parts.pop() as string;
    let node = root;
    let acc = "";
    for (const part of parts) {
      acc = acc ? `${acc}/${part}` : part;
      node = ensure(node, part, acc);
    }
    node.files.push(file);
  }
  // SKILL.md always floats to the top of the root list.
  root.files.sort((a, b) => {
    if (a.path === "SKILL.md") return -1;
    if (b.path === "SKILL.md") return 1;
    return a.path.localeCompare(b.path);
  });
  root.folders.sort((a, b) => a.name.localeCompare(b.name));
  return root;
}

function FolderNode({
  folder,
  depth,
  activePath,
  onSelect,
  onDelete,
}: {
  folder: TreeFolder;
  depth: number;
  activePath: string;
  onSelect: (path: string) => void;
  onDelete?: (path: string) => void;
}) {
  const [open, setOpen] = React.useState(true);
  return (
    <li>
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-1.5 rounded px-2 py-1 text-left font-mono text-label text-muted-foreground hover:bg-surface-2"
        style={{ paddingLeft: 8 + depth * 14 }}
      >
        <ChevronRight size={13} className={cn("shrink-0 transition-transform", open && "rotate-90")} />
        <FolderOpen size={13} className="shrink-0 text-faint" />
        <span className="truncate">{folder.name}</span>
      </button>
      {open && (
        <ul>
          {folder.folders.map((sub) => (
            <FolderNode
              key={sub.path}
              folder={sub}
              depth={depth + 1}
              activePath={activePath}
              onSelect={onSelect}
              onDelete={onDelete}
            />
          ))}
          {folder.files.map((file) => (
            <FileNode
              key={file.path}
              file={file}
              depth={depth + 1}
              activePath={activePath}
              onSelect={onSelect}
              onDelete={onDelete}
            />
          ))}
        </ul>
      )}
    </li>
  );
}

function FileNode({
  file,
  depth,
  activePath,
  onSelect,
  onDelete,
}: {
  file: SkillFile;
  depth: number;
  activePath: string;
  onSelect: (path: string) => void;
  onDelete?: (path: string) => void;
}) {
  const Icon = iconFor(file);
  const active = file.path === activePath;
  const isRoot = file.path === "SKILL.md";
  const name = file.path.split("/").pop();
  return (
    <li className="group/file relative">
      <button
        type="button"
        onClick={() => onSelect(file.path)}
        className={cn(
          "flex w-full items-center gap-1.5 rounded px-2 py-1 text-left font-mono text-label",
          active ? "bg-[var(--ox-tint)] text-ox" : "text-ink hover:bg-surface-2",
        )}
        style={{ paddingLeft: 8 + depth * 14 + 13 }}
      >
        <Icon size={13} className={cn("shrink-0", active ? "text-ox" : "text-faint")} />
        <span className="truncate">{name}</span>
      </button>
      {onDelete && !isRoot && (
        <button
          type="button"
          onClick={() => onDelete(file.path)}
          aria-label={`Delete ${file.path}`}
          className="absolute right-1 top-1/2 hidden -translate-y-1/2 rounded p-1 text-faint hover:text-ox group-hover/file:block"
        >
          <Trash2 size={12} />
        </button>
      )}
    </li>
  );
}

export function FileTree({
  files,
  activePath,
  onSelect,
  onAddFile,
  onDeleteFile,
}: {
  files: SkillFile[];
  activePath: string;
  onSelect: (path: string) => void;
  /** Returns false if the path collides or is invalid (caller surfaces the reason). */
  onAddFile?: (path: string) => boolean;
  onDeleteFile?: (path: string) => void;
}) {
  const tree = React.useMemo(() => buildTree(files), [files]);
  const [adding, setAdding] = React.useState(false);
  const [draft, setDraft] = React.useState("");
  const [err, setErr] = React.useState<string | null>(null);

  const commit = () => {
    const path = draft.trim();
    if (!path) {
      setAdding(false);
      setDraft("");
      setErr(null);
      return;
    }
    if (files.some((f) => f.path === path)) {
      setErr("A file with that path already exists.");
      return;
    }
    const ok = onAddFile?.(path) ?? false;
    if (ok) {
      setAdding(false);
      setDraft("");
      setErr(null);
    } else {
      setErr("Could not add that file.");
    }
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between px-2 pb-2">
        <span className="rail-group-label">files</span>
        {onAddFile && (
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6"
            aria-label="Add file"
            onClick={() => {
              setAdding(true);
              setErr(null);
            }}
          >
            <Plus size={13} />
          </Button>
        )}
      </div>
      <ul className="min-h-0 flex-1 overflow-y-auto pb-2">
        {tree.folders.map((folder) => (
          <FolderNode
            key={folder.path}
            folder={folder}
            depth={0}
            activePath={activePath}
            onSelect={onSelect}
            onDelete={onDeleteFile}
          />
        ))}
        {tree.files.map((file) => (
          <FileNode
            key={file.path}
            file={file}
            depth={0}
            activePath={activePath}
            onSelect={onSelect}
            onDelete={onDeleteFile}
          />
        ))}
      </ul>
      {adding && (
        <div className="border-t border-line px-2 pt-2">
          <Input
            autoFocus
            value={draft}
            placeholder="scripts/run.py"
            className="h-8 font-mono text-label"
            onChange={(e) => {
              setDraft(e.target.value);
              setErr(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") {
                setAdding(false);
                setDraft("");
                setErr(null);
              }
            }}
            onBlur={commit}
          />
          {err && <p className="mt-1 font-mono text-[11px] text-ox">{err}</p>}
        </div>
      )}
    </div>
  );
}
