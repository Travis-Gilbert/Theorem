"use client";

import * as React from "react";
import { AtSign, Activity, Bell, Briefcase, Check, Archive, ExternalLink, Mail } from "lucide-react";
import Link from "next/link";
import { type InboxItem, type InboxKind } from "@/lib/harness";
import { cn, relativeTime } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/common/EmptyState";

const KIND_ICON: Record<InboxKind, React.ComponentType<{ size?: number; className?: string }>> = {
  mention: AtSign,
  run: Activity,
  system: Bell,
  job: Briefcase,
};

/** The inbox list (left pane). Unread rows carry an oxblood dot + heavier title. */
export function InboxList({
  items,
  selectedId,
  onSelect,
}: {
  items: InboxItem[];
  selectedId: string | null;
  onSelect: (item: InboxItem) => void;
}) {
  if (!items.length) {
    return <EmptyState icon={Mail} title="Inbox zero" description="No mentions, runs, or alerts waiting." />;
  }
  return (
    <ul className="flex flex-col divide-y divide-line">
      {items.map((item) => {
        const Icon = KIND_ICON[item.kind];
        const selected = item.id === selectedId;
        return (
          <li key={item.id}>
            <button
              onClick={() => onSelect(item)}
              data-selected={selected}
              className={cn(
                "flex w-full flex-col gap-1 px-4 py-3 text-left transition-colors hover:bg-surface",
                selected && "bg-[var(--ox-tint)]",
              )}
            >
              <div className="flex items-center gap-2">
                <span className={cn("status-dot", item.read ? "opacity-0" : "")} style={{ background: "var(--ox)" }} />
                <Icon size={13} className="text-muted-foreground" />
                <span className={cn("flex-1 truncate text-body", item.read ? "text-ink" : "font-semibold text-ink")}>
                  {item.title}
                </span>
                <span className="shrink-0 font-mono text-[10px] text-faint">{relativeTime(item.at)}</span>
              </div>
              <p className="truncate pl-7 text-label text-muted-foreground">{item.preview}</p>
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/** The reading pane (right). Shows the body + actions. */
export function InboxReader({
  item,
  onMarkRead,
  onArchive,
}: {
  item: InboxItem | null;
  onMarkRead: (id: string, read: boolean) => void;
  onArchive: (id: string) => void;
}) {
  if (!item) {
    return (
      <div className="grid h-full place-items-center font-mono text-label text-muted-foreground">
        Select an item to read it.
      </div>
    );
  }
  const Icon = KIND_ICON[item.kind];
  return (
    <div className="flex h-full flex-col">
      <div className="flex items-start justify-between gap-3 border-b border-line p-5">
        <div className="min-w-0">
          <div className="mb-1 flex items-center gap-2 font-mono text-[11px] text-muted-foreground">
            <Icon size={13} /> {item.from}
            {item.room && <span className="text-faint">&middot; {item.room}</span>}
            <span className="text-faint">&middot; {relativeTime(item.at)}</span>
          </div>
          <h2 className="font-title text-subhead text-ink">{item.title}</h2>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <Button variant="ghost" size="sm" onClick={() => onMarkRead(item.id, !item.read)}>
            <Check size={14} /> {item.read ? "Unread" : "Read"}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => onArchive(item.id)}>
            <Archive size={14} /> Archive
          </Button>
        </div>
      </div>
      <div className="flex-1 overflow-y-auto p-5">
        <p className="max-w-[var(--measure)] text-body leading-relaxed text-ink">{item.body}</p>
      </div>
      {item.href && (
        <div className="border-t border-line p-4">
          <Button variant="outline" size="sm" asChild>
            <Link href={item.href}>
              <ExternalLink size={14} /> Open source
            </Link>
          </Button>
        </div>
      )}
    </div>
  );
}
