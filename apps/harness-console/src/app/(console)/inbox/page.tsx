"use client";

import * as React from "react";
import { harness, type InboxItem } from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";
import { cn } from "@/lib/utils";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/misc";
import { InboxList, InboxReader } from "@/components/inbox/InboxPane";
import { TaskBoard } from "@/components/inbox/TaskBoard";

type Tab = "inbox" | "tasks";
type Filter = "all" | "unread" | "mention";

const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "unread", label: "Unread" },
  { id: "mention", label: "Mentions" },
];

export default function InboxPage() {
  const [tab, setTab] = React.useState<Tab>("inbox");
  const [filter, setFilter] = React.useState<Filter>("all");
  const [selectedId, setSelectedId] = React.useState<string | null>(null);

  const { data: inbox, loading: inboxLoading, reload: reloadInbox } = useAsync(() => harness.listInbox());
  const { data: tasks, loading: tasksLoading, reload: reloadTasks } = useAsync(() => harness.listTasks());

  const items = React.useMemo<InboxItem[]>(() => {
    const all = inbox ?? [];
    if (filter === "unread") return all.filter((i) => !i.read);
    if (filter === "mention") return all.filter((i) => i.kind === "mention");
    return all;
  }, [inbox, filter]);

  const selected = React.useMemo(
    () => (inbox ?? []).find((i) => i.id === selectedId) ?? null,
    [inbox, selectedId],
  );

  const unreadCount = (inbox ?? []).filter((i) => !i.read).length;

  const onSelect = async (item: InboxItem) => {
    setSelectedId(item.id);
    if (!item.read) {
      await harness.markInboxRead(item.id, true);
      reloadInbox();
    }
  };
  const onMarkRead = async (id: string, read: boolean) => {
    await harness.markInboxRead(id, read);
    reloadInbox();
  };
  const onArchive = async (id: string) => {
    await harness.archiveInboxItem(id);
    if (selectedId === id) setSelectedId(null);
    reloadInbox();
  };
  const onMove = async (id: string, state: Parameters<typeof harness.updateTaskState>[1]) => {
    await harness.updateTaskState(id, state);
    reloadTasks();
  };

  return (
    <div className="flex h-full flex-col">
      {/* Full-bleed header */}
      <header className="flex items-center justify-between gap-4 border-b border-line px-5 py-3">
        <div className="flex items-center gap-2">
          <h1 className="font-title text-title text-ink">Inbox</h1>
          {unreadCount > 0 && <Badge tone="accent">{unreadCount} unread</Badge>}
        </div>
        <Tabs value={tab} onValueChange={(v) => setTab(v as Tab)}>
          <TabsList>
            <TabsTrigger value="inbox">Inbox</TabsTrigger>
            <TabsTrigger value="tasks">Tasks</TabsTrigger>
          </TabsList>
        </Tabs>
      </header>

      {tab === "inbox" ? (
        <div className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-[minmax(0,380px)_1fr]">
          {/* List pane */}
          <div className="flex min-h-0 flex-col border-r border-line">
            <div className="flex items-center gap-1 border-b border-line px-3 py-2">
              {FILTERS.map((f) => (
                <button
                  key={f.id}
                  onClick={() => setFilter(f.id)}
                  className={cn(
                    "rounded px-2 py-1 font-mono text-[11px]",
                    filter === f.id ? "bg-[var(--ox-tint)] text-ox" : "text-muted-foreground hover:text-ink",
                  )}
                >
                  {f.label}
                </button>
              ))}
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              {inboxLoading ? (
                <div className="flex flex-col gap-2 p-4">
                  {Array.from({ length: 6 }).map((_, i) => (
                    <Skeleton key={i} className="h-12 w-full" />
                  ))}
                </div>
              ) : (
                <InboxList items={items} selectedId={selectedId} onSelect={onSelect} />
              )}
            </div>
          </div>
          {/* Reader pane */}
          <div className="hidden min-h-0 md:block">
            <InboxReader item={selected} onMarkRead={onMarkRead} onArchive={onArchive} />
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1">
          {tasksLoading ? (
            <div className="grid grid-cols-1 gap-3 p-4 md:grid-cols-4">
              {Array.from({ length: 4 }).map((_, i) => (
                <Skeleton key={i} className="h-40 w-full" />
              ))}
            </div>
          ) : (
            <TaskBoard tasks={tasks ?? []} onMove={onMove} />
          )}
        </div>
      )}
    </div>
  );
}
