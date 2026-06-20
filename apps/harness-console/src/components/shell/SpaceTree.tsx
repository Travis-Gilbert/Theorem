"use client";

import * as React from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { ChevronRight, FolderOpen, GripVertical, Pencil } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { getSpaceTypeDefinition } from "@/lib/spaces/registry";
import type { SpaceTypeInstance } from "@/lib/spaces/types";

interface SpaceTreeProps {
  spaces: SpaceTypeInstance[];
  horizontal?: boolean;
  onRename: (id: string, label: string) => void;
  onReorder: (activeId: string, overId: string) => void;
}

export function SpaceTree({ spaces, horizontal = false, onRename, onReorder }: SpaceTreeProps) {
  const pathname = usePathname() ?? "";
  const visible = React.useMemo(() => spaces.filter((space) => space.enabled), [spaces]);
  const roots = React.useMemo(
    () => visible.filter((space) => !space.parent).sort(byOrder),
    [visible],
  );
  const childMap = React.useMemo(() => {
    const map = new Map<string, SpaceTypeInstance[]>();
    for (const space of visible) {
      if (!space.parent) continue;
      const children = map.get(space.parent) ?? [];
      children.push(space);
      map.set(space.parent, children);
    }
    for (const children of map.values()) {
      children.sort(byOrder);
    }
    return map;
  }, [visible]);
  const [dragging, setDragging] = React.useState<string | null>(null);

  if (horizontal) {
    const flat = flattenSpaces(roots, childMap).filter((space) => definitionHref(space));
    return (
      <nav className="rail-shell flex w-full items-center gap-1 overflow-x-auto border-b border-line bg-surface px-2 py-2">
        {flat.map((space) => (
          <div key={space.id} className="shrink-0">
            <SpaceRow
              space={space}
              active={isActive(pathname, definitionHref(space))}
              depth={0}
              childMap={childMap}
              dragging={dragging}
              setDragging={setDragging}
              onRename={onRename}
              onReorder={onReorder}
              horizontal
            />
          </div>
        ))}
      </nav>
    );
  }

  return (
    <TooltipProvider delayDuration={350}>
      <nav className="flex flex-1 flex-col gap-0.5 overflow-y-auto px-3 pb-2">
        <ul className="space-y-0.5">
          {roots.map((space) => (
            <SpaceNode
              key={space.id}
              space={space}
              depth={0}
              pathname={pathname}
              childMap={childMap}
              dragging={dragging}
              setDragging={setDragging}
              onRename={onRename}
              onReorder={onReorder}
            />
          ))}
        </ul>
      </nav>
    </TooltipProvider>
  );
}

function SpaceNode({
  space,
  depth,
  pathname,
  childMap,
  dragging,
  setDragging,
  onRename,
  onReorder,
}: {
  space: SpaceTypeInstance;
  depth: number;
  pathname: string;
  childMap: Map<string, SpaceTypeInstance[]>;
  dragging: string | null;
  setDragging: (id: string | null) => void;
  onRename: (id: string, label: string) => void;
  onReorder: (activeId: string, overId: string) => void;
}) {
  const children = childMap.get(space.id) ?? [];
  const [open, setOpen] = React.useState(true);
  const href = definitionHref(space);

  return (
    <li>
      <SpaceRow
        space={space}
        active={isActive(pathname, href)}
        depth={depth}
        childMap={childMap}
        open={open}
        setOpen={children.length ? setOpen : undefined}
        dragging={dragging}
        setDragging={setDragging}
        onRename={onRename}
        onReorder={onReorder}
      />
      <div
        className={cn(
          "grid transition-[grid-template-rows,opacity] duration-150 ease-out",
          open ? "grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
        )}
      >
        <ul className="min-h-0 overflow-hidden">
          {children.map((child) => (
            <SpaceNode
              key={child.id}
              space={child}
              depth={depth + 1}
              pathname={pathname}
              childMap={childMap}
              dragging={dragging}
              setDragging={setDragging}
              onRename={onRename}
              onReorder={onReorder}
            />
          ))}
        </ul>
      </div>
    </li>
  );
}

function SpaceRow({
  space,
  active,
  depth,
  childMap,
  open,
  setOpen,
  dragging,
  setDragging,
  onRename,
  onReorder,
  horizontal = false,
}: {
  space: SpaceTypeInstance;
  active: boolean;
  depth: number;
  childMap: Map<string, SpaceTypeInstance[]>;
  open?: boolean;
  setOpen?: (open: boolean) => void;
  dragging: string | null;
  setDragging: (id: string | null) => void;
  onRename: (id: string, label: string) => void;
  onReorder: (activeId: string, overId: string) => void;
  horizontal?: boolean;
}) {
  const definition = getSpaceTypeDefinition(space.typeKey);
  const Icon = definition?.icon ?? FolderOpen;
  const href = definition?.href;
  const children = childMap.get(space.id) ?? [];
  const [editing, setEditing] = React.useState(false);
  const [draft, setDraft] = React.useState(space.label);

  const commitRename = () => {
    const next = draft.trim();
    setEditing(false);
    if (next && next !== space.label) {
      onRename(space.id, next);
    } else {
      setDraft(space.label);
    }
  };

  const content = (
    <>
      <span className="grid w-4 place-items-center">
        {children.length > 0 ? (
          <ChevronRight
            size={13}
            className={cn("transition-transform", open && "rotate-90")}
            onClick={(event) => {
              event.preventDefault();
              setOpen?.(!open);
            }}
          />
        ) : (
          <GripVertical size={12} className="text-faint" />
        )}
      </span>
      <Icon size={15} className={active ? "text-ox" : "text-faint"} />
      {editing ? (
        <Input
          value={draft}
          autoFocus
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commitRename}
          onKeyDown={(event) => {
            if (event.key === "Enter") commitRename();
            if (event.key === "Escape") {
              setDraft(space.label);
              setEditing(false);
            }
          }}
          className="h-6 min-w-0 border-line bg-bg px-1.5 py-0 font-mono text-label"
        />
      ) : (
        <span className="min-w-0 flex-1 truncate">{space.label}</span>
      )}
      {!horizontal && !editing && (
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              aria-label={`Rename ${space.label}`}
              onClick={(event) => {
                event.preventDefault();
                setDraft(space.label);
                setEditing(true);
              }}
              className="grid h-5 w-5 place-items-center rounded text-faint opacity-0 hover:text-ox group-hover/space:opacity-100"
            >
              <Pencil size={11} />
            </button>
          </TooltipTrigger>
          <TooltipContent>Rename</TooltipContent>
        </Tooltip>
      )}
    </>
  );

  const className = cn(
    "group/space relative flex w-full items-center gap-1.5 rounded-md py-1.5 pr-2 font-mono text-label transition-colors",
    horizontal ? "px-2.5" : "pl-2",
    active
      ? "bg-[var(--ox-tint)] text-ink before:absolute before:inset-y-1 before:left-0 before:w-0.5 before:rounded-full before:bg-ox before:content-['']"
      : "text-muted-foreground hover:bg-surface-2 hover:text-ink",
    dragging === space.id && "opacity-50",
  );
  const style = horizontal ? undefined : { paddingLeft: 8 + depth * 16 };
  const dragProps = {
    draggable: true,
    onDragStart: () => setDragging(space.id),
    onDragEnd: () => setDragging(null),
    onDragOver: (event: React.DragEvent) => event.preventDefault(),
    onDrop: (event: React.DragEvent) => {
      event.preventDefault();
      if (dragging && dragging !== space.id) {
        onReorder(dragging, space.id);
      }
      setDragging(null);
    },
  };

  if (href && !editing) {
    return (
      <Link
        href={href}
        aria-current={active ? "page" : undefined}
        className={className}
        style={style}
        onDoubleClick={() => setEditing(true)}
        {...dragProps}
      >
        {content}
      </Link>
    );
  }

  return (
    <button
      type="button"
      className={className}
      style={style}
      onClick={() => setOpen?.(!open)}
      onDoubleClick={() => setEditing(true)}
      {...dragProps}
    >
      {content}
    </button>
  );
}

function definitionHref(space: SpaceTypeInstance): string | undefined {
  return getSpaceTypeDefinition(space.typeKey)?.href;
}

function isActive(pathname: string, href?: string): boolean {
  if (!href) return false;
  if (href === "/") return pathname === "/";
  return pathname === href || pathname.startsWith(`${href}/`);
}

function byOrder(a: SpaceTypeInstance, b: SpaceTypeInstance): number {
  return a.order - b.order || a.label.localeCompare(b.label);
}

function flattenSpaces(
  roots: SpaceTypeInstance[],
  childMap: Map<string, SpaceTypeInstance[]>,
): SpaceTypeInstance[] {
  return roots.flatMap((space) => [space, ...flattenSpaces(childMap.get(space.id) ?? [], childMap)]);
}
