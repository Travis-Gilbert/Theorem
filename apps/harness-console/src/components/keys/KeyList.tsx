"use client";

import * as React from "react";
import { KeyRound, Plus, Trash2 } from "lucide-react";
import { harness, type HarnessKey } from "@/lib/harness";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/misc";
import {
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogClose,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/misc";
import { EmptyState } from "@/components/common/EmptyState";
import { toast } from "@/components/ui/toaster";
import { useAsync } from "@/lib/hooks/useAsync";
import { relativeTime } from "@/lib/utils";

/** The scope vocabulary the harness understands, grouped for the picker. */
const SCOPE_GROUPS: { group: string; scopes: string[] }[] = [
  { group: "memory", scopes: ["memory:read", "memory:write"] },
  { group: "coordination", scopes: ["coordination:read", "coordination:write"] },
  { group: "run", scopes: ["run:read", "run:write"] },
  { group: "skills", scopes: ["skills:read", "skills:write"] },
];
const DEFAULT_SCOPES = ["memory:read", "memory:write", "coordination:read", "run:write"];

/**
 * The inbound key list. Each key shows its human name, the displayable prefix
 * (the secret half is shown once at mint and never again), when it was created,
 * last-used relative time, its scope chips, and a revoke action. A create-key
 * dialog mints a new key with a chosen scope set and reveals it once.
 */
export function KeyList({
  onKeyCreated,
}: {
  /** Called with the freshly-minted key so the page can route the install panel to it. */
  onKeyCreated?: (key: HarnessKey) => void;
}) {
  const { data: keys, loading, error, reload } = useAsync(() => harness.listKeys(), []);

  async function revoke(key: HarnessKey) {
    await harness.revokeKey(key.id);
    toast.success(`Revoked "${key.name}"`);
    reload();
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <p className="text-label text-muted-foreground">
          {keys ? `${keys.length} active ${keys.length === 1 ? "key" : "keys"}` : "Loading keys"}
        </p>
        <CreateKeyDialog
          onCreated={(k) => {
            reload();
            onKeyCreated?.(k);
          }}
        />
      </div>

      {loading ? (
        <KeyListSkeleton />
      ) : error ? (
        <EmptyState
          icon={KeyRound}
          title="Couldn't load keys"
          description={error}
          action={
            <Button variant="outline" size="sm" onClick={reload}>
              Try again
            </Button>
          }
        />
      ) : !keys || keys.length === 0 ? (
        <EmptyState
          icon={KeyRound}
          title="No keys yet"
          description="Mint a key to connect Claude Code, Codex, Gemini, or a raw client to the harness."
          action={
            <CreateKeyDialog
              onCreated={(k) => {
                reload();
                onKeyCreated?.(k);
              }}
              trigger={
                <Button variant="primary" size="sm">
                  <Plus size={14} /> Create your first key
                </Button>
              }
            />
          }
        />
      ) : (
        <div className="material-blueprint material overflow-hidden p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Prefix</TableHead>
                <TableHead>Created</TableHead>
                <TableHead>Last used</TableHead>
                <TableHead>Scopes</TableHead>
                <TableHead className="text-right">Revoke</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {keys.map((k) => (
                <TableRow key={k.id}>
                  <TableCell className="font-medium text-ink">{k.name}</TableCell>
                  <TableCell className="font-mono text-label text-muted-foreground">{k.prefix}</TableCell>
                  <TableCell className="font-mono text-label text-muted-foreground">{relativeTime(k.created)}</TableCell>
                  <TableCell className="font-mono text-label text-muted-foreground">
                    {k.lastUsed ? relativeTime(k.lastUsed) : "never"}
                  </TableCell>
                  <TableCell>
                    <div className="flex max-w-sm flex-wrap gap-1">
                      {k.scopes.map((s) => (
                        <Badge key={s} tone="neutral">
                          {s}
                        </Badge>
                      ))}
                    </div>
                  </TableCell>
                  <TableCell className="text-right">
                    <RevokeButton keyItem={k} onConfirm={() => revoke(k)} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}

function KeyListSkeleton() {
  return (
    <div className="material p-4">
      <div className="space-y-3">
        {Array.from({ length: 3 }).map((_, i) => (
          <div key={i} className="flex items-center gap-4">
            <Skeleton className="h-4 w-40" />
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-4 w-16" />
            <Skeleton className="h-4 w-16" />
            <Skeleton className="ml-auto h-6 w-48" />
          </div>
        ))}
      </div>
    </div>
  );
}

function RevokeButton({ keyItem, onConfirm }: { keyItem: HarnessKey; onConfirm: () => void }) {
  const [open, setOpen] = React.useState(false);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="danger" size="sm" aria-label={`Revoke ${keyItem.name}`}>
          <Trash2 size={13} /> Revoke
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Revoke this key?</DialogTitle>
          <DialogDescription>
            Any client using <span className="font-mono text-ink">{keyItem.prefix}</span> ({keyItem.name}) will lose
            access immediately. This cannot be undone.
          </DialogDescription>
        </DialogHeader>
        <div className="flex justify-end gap-2">
          <DialogClose asChild>
            <Button variant="outline" size="sm">
              Cancel
            </Button>
          </DialogClose>
          <Button
            variant="primary"
            size="sm"
            onClick={() => {
              onConfirm();
              setOpen(false);
            }}
          >
            Revoke key
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function CreateKeyDialog({
  onCreated,
  trigger,
}: {
  onCreated: (key: HarnessKey) => void;
  trigger?: React.ReactNode;
}) {
  const [open, setOpen] = React.useState(false);
  const [name, setName] = React.useState("");
  const [scopes, setScopes] = React.useState<string[]>(DEFAULT_SCOPES);
  const [submitting, setSubmitting] = React.useState(false);

  function toggleScope(scope: string) {
    setScopes((prev) => (prev.includes(scope) ? prev.filter((s) => s !== scope) : [...prev, scope]));
  }

  function reset() {
    setName("");
    setScopes(DEFAULT_SCOPES);
    setSubmitting(false);
  }

  async function submit() {
    const trimmed = name.trim();
    if (!trimmed) {
      toast.error("Give the key a name");
      return;
    }
    if (scopes.length === 0) {
      toast.error("Pick at least one scope");
      return;
    }
    setSubmitting(true);
    try {
      const key = await harness.createKey(trimmed, scopes);
      toast.success(`Created "${key.name}"`);
      onCreated(key);
      setOpen(false);
      reset();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Could not create the key");
      setSubmitting(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        setOpen(o);
        if (!o) reset();
      }}
    >
      <DialogTrigger asChild>
        {trigger ?? (
          <Button variant="primary" size="sm">
            <Plus size={14} /> Create key
          </Button>
        )}
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create a key</DialogTitle>
          <DialogDescription>
            Name the client and choose its scopes. The tenant is bound to the key, so a client only ever pastes the URL
            and the key.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="key-name">Name</Label>
            <Input
              id="key-name"
              autoFocus
              placeholder="Claude Code (laptop)"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") submit();
              }}
            />
          </div>
          <div className="space-y-2">
            <Label>Scopes</Label>
            <div className="space-y-2">
              {SCOPE_GROUPS.map((g) => (
                <div key={g.group} className="flex flex-wrap items-center gap-1.5">
                  <span className="w-28 shrink-0 font-mono text-[11px] uppercase tracking-wide text-faint">
                    {g.group}
                  </span>
                  {g.scopes.map((s) => {
                    const on = scopes.includes(s);
                    return (
                      <button
                        key={s}
                        type="button"
                        onClick={() => toggleScope(s)}
                        aria-pressed={on}
                        className={
                          "rounded border px-2 py-0.5 font-mono text-[11px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--ox-ring)] " +
                          (on
                            ? "border-[var(--ox)] bg-[var(--ox-tint)] text-ox"
                            : "border-line bg-surface-2 text-muted-foreground hover:text-ink")
                        }
                      >
                        {s}
                      </button>
                    );
                  })}
                </div>
              ))}
            </div>
          </div>
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <DialogClose asChild>
            <Button variant="outline" size="sm" disabled={submitting}>
              Cancel
            </Button>
          </DialogClose>
          <Button variant="primary" size="sm" onClick={submit} disabled={submitting}>
            {submitting ? "Creating..." : "Create key"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
