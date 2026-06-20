"use client";

import * as React from "react";
import Link from "next/link";
import { Paperclip, ArrowUp, X, FileText, Hash, Loader2, KeyRound, Search } from "lucide-react";
import { harness, type Atom, type Room } from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { Skeleton } from "@/components/ui/misc";
import { cn } from "@/lib/utils";

/**
 * An attachment is a scope reference passed to runAgent(prompt, scope): a memory
 * atom ("atom:<id>") or a room ("room:<id>"). The agent reads the attached
 * context into the turn rather than the user pasting it.
 */
export interface Attachment {
  ref: string; // "atom:atom_001" | "room:room_console"
  kind: "atom" | "room";
  label: string;
}

function AttachPicker({
  attached,
  onAdd,
}: {
  attached: Attachment[];
  onAdd: (a: Attachment) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const [q, setQ] = React.useState("");
  const { data: memory, loading: loadingAtoms } = useAsync(() => harness.listMemory(), []);
  const { data: rooms, loading: loadingRooms } = useAsync(() => harness.listRooms(), []);

  const attachedRefs = new Set(attached.map((a) => a.ref));
  const term = q.trim().toLowerCase();

  const atoms = (memory?.atoms ?? [])
    .filter((a) => !attachedRefs.has(`atom:${a.id}`))
    .filter((a) => !term || `${a.title} ${a.summary}`.toLowerCase().includes(term))
    .slice(0, 8);
  const roomList = (rooms ?? [])
    .filter((r) => !attachedRefs.has(`room:${r.id}`))
    .filter((r) => !term || `${r.name} ${r.topic}`.toLowerCase().includes(term));

  function addAtom(a: Atom) {
    onAdd({ ref: `atom:${a.id}`, kind: "atom", label: a.title });
    setOpen(false);
    setQ("");
  }
  function addRoom(r: Room) {
    onAdd({ ref: `room:${r.id}`, kind: "room", label: r.name });
    setOpen(false);
    setQ("");
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button type="button" variant="ghost" size="icon" aria-label="Attach context">
          <Paperclip size={16} />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 p-0">
        <div className="border-b border-line p-2">
          <div className="flex items-center gap-2 rounded border border-line bg-bg px-2">
            <Search size={13} className="text-faint" />
            <input
              autoFocus
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder="Attach an atom or room"
              className="h-8 w-full bg-transparent text-label text-ink placeholder:text-faint focus:outline-none"
            />
          </div>
        </div>
        <Tabs defaultValue="atoms">
          <TabsList className="m-2 mb-0">
            <TabsTrigger value="atoms">
              <FileText size={12} /> atoms
            </TabsTrigger>
            <TabsTrigger value="rooms">
              <Hash size={12} /> rooms
            </TabsTrigger>
          </TabsList>
          <TabsContent value="atoms" className="max-h-64 overflow-y-auto p-1">
            {loadingAtoms ? (
              <div className="space-y-1 p-1">
                <Skeleton className="h-9 w-full" />
                <Skeleton className="h-9 w-full" />
              </div>
            ) : atoms.length === 0 ? (
              <p className="px-2 py-3 text-center font-mono text-[11px] text-faint">no matching atoms</p>
            ) : (
              atoms.map((a) => (
                <button
                  key={a.id}
                  type="button"
                  onClick={() => addAtom(a)}
                  className="flex w-full items-start gap-2 rounded px-2 py-1.5 text-left hover:bg-surface-2"
                >
                  <FileText size={13} className="mt-0.5 shrink-0 text-muted-foreground" />
                  <span className="min-w-0">
                    <span className="block truncate text-label text-ink">{a.title}</span>
                    <span className="block truncate font-mono text-[11px] text-faint">{a.kind}</span>
                  </span>
                </button>
              ))
            )}
          </TabsContent>
          <TabsContent value="rooms" className="max-h-64 overflow-y-auto p-1">
            {loadingRooms ? (
              <div className="space-y-1 p-1">
                <Skeleton className="h-9 w-full" />
              </div>
            ) : roomList.length === 0 ? (
              <p className="px-2 py-3 text-center font-mono text-[11px] text-faint">no matching rooms</p>
            ) : (
              roomList.map((r) => (
                <button
                  key={r.id}
                  type="button"
                  onClick={() => addRoom(r)}
                  className="flex w-full items-start gap-2 rounded px-2 py-1.5 text-left hover:bg-surface-2"
                >
                  <Hash size={13} className="mt-0.5 shrink-0 text-muted-foreground" />
                  <span className="min-w-0">
                    <span className="block truncate text-label text-ink">{r.name}</span>
                    <span className="block truncate font-mono text-[11px] text-faint">{r.topic}</span>
                  </span>
                </button>
              ))
            )}
          </TabsContent>
        </Tabs>
      </PopoverContent>
    </Popover>
  );
}

export function Composer({
  value,
  onChange,
  attachments,
  onAttach,
  onRemoveAttachment,
  onSend,
  sending,
  disabled = false,
  disabledReason,
  className,
}: {
  value: string;
  onChange: (v: string) => void;
  attachments: Attachment[];
  onAttach: (a: Attachment) => void;
  onRemoveAttachment: (ref: string) => void;
  onSend: () => void;
  sending: boolean;
  disabled?: boolean;
  disabledReason?: React.ReactNode;
  className?: string;
}) {
  const canSend = !disabled && !sending && value.trim().length > 0;

  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      if (canSend) onSend();
    }
  }

  if (disabled) {
    return (
      <div className={cn("material flex flex-col items-center gap-3 p-5 text-center", className)}>
        <span className="grid h-10 w-10 place-items-center rounded-full bg-surface-2 text-[var(--warn)]">
          <KeyRound size={18} />
        </span>
        <div>
          <p className="font-title text-subhead text-ink">No provider key, no run</p>
          <p className="mt-1 max-w-sm text-label text-muted-foreground">
            {disabledReason ??
              "Every head in the composed agent is missing a provider key. Add one to start tasking the agent."}
          </p>
        </div>
        <Button asChild variant="primary" size="sm">
          <Link href="/providers">
            <KeyRound size={13} /> Add a provider key
          </Link>
        </Button>
      </div>
    );
  }

  return (
    <div className={cn("material p-3", className)}>
      {attachments.length > 0 && (
        <div className="mb-2 flex flex-wrap gap-1.5">
          {attachments.map((a) => (
            <Badge key={a.ref} tone="accent" className="gap-1 pr-1">
              {a.kind === "atom" ? <FileText size={11} /> : <Hash size={11} />}
              <span className="max-w-40 truncate">{a.label}</span>
              <button
                type="button"
                onClick={() => onRemoveAttachment(a.ref)}
                aria-label={`Remove ${a.label}`}
                className="ml-0.5 rounded p-0.5 hover:bg-[var(--ox-tint)]"
              >
                <X size={11} />
              </button>
            </Badge>
          ))}
        </div>
      )}

      <Textarea
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        rows={3}
        placeholder="Ask the Theorem agent. The heads run as peers over your scope."
        className="resize-none border-0 bg-transparent px-1 focus-visible:ring-0"
        disabled={sending}
      />

      <div className="mt-2 flex items-center gap-2">
        <AttachPicker attached={attachments} onAdd={onAttach} />
        <span className="font-mono text-[11px] text-faint">
          {sending ? "running" : "Cmd+Enter to send"}
        </span>
        <Button
          type="button"
          variant="primary"
          size="sm"
          className="ml-auto"
          disabled={!canSend}
          onClick={onSend}
        >
          {sending ? <Loader2 size={14} className="animate-spin" /> : <ArrowUp size={14} />}
          Send
        </Button>
      </div>
    </div>
  );
}
