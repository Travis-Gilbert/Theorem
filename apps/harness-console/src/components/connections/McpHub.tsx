"use client";

import * as React from "react";
import {
  Plug,
  DoorOpen,
  Server,
  Copy,
  Check,
  Plus,
  Trash2,
  RefreshCw,
  AlertTriangle,
  Layers,
} from "lucide-react";
import {
  harness,
  type McpHubState,
  type CapabilityNamespace,
  type BrokeredServer,
  type ClientKind,
} from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { StatusDot } from "@/components/ui/status-dot";
import { Switch, Label, Skeleton, Separator } from "@/components/ui/misc";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogClose,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from "@/components/ui/select";
import { RetroFrame, GlowCard } from "@/components/retro/retro";
import { EmptyState } from "@/components/common/EmptyState";
import { toast } from "@/components/ui/toaster";
import { cn } from "@/lib/utils";

/**
 * The MCP Hub: the harness is the single MCP endpoint coding agents connect to.
 * One front door (the GlowCard hero) aggregates the harness's own namespaced
 * verbs and brokers other MCP servers behind that one connection.
 *
 * Three controls: (1) toggle capability namespaces on/off, (2) register/remove
 * brokered MCP servers, (3) copy the one-connection install snippet per client.
 */

// --- Connection snippet (one front door, many clients) --------------------

const CLIENTS: { kind: ClientKind; label: string }[] = [
  { kind: "claude", label: "Claude Code" },
  { kind: "codex", label: "Codex" },
  { kind: "gemini", label: "Gemini" },
];

const PLACEHOLDER_KEY = "<your-key>";

function SnippetBlock({ client }: { client: ClientKind }) {
  const snippet = React.useMemo(() => harness.installSnippet(client, PLACEHOLDER_KEY), [client]);
  const [copied, setCopied] = React.useState(false);

  const copy = React.useCallback(async () => {
    try {
      await navigator.clipboard.writeText(snippet);
      setCopied(true);
      toast.success("Connection snippet copied");
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      toast.error("Could not copy to clipboard");
    }
  }, [snippet]);

  return (
    <div className="relative">
      <pre className="overflow-x-auto rounded-md border border-line bg-surface-2 p-3 pr-12 font-mono text-[12px] leading-relaxed text-ink">
        {snippet}
      </pre>
      <Button
        variant="ghost"
        size="icon"
        className="absolute right-2 top-2 h-7 w-7"
        onClick={copy}
        aria-label="Copy snippet"
      >
        {copied ? <Check size={14} className="text-[var(--live)]" /> : <Copy size={14} />}
      </Button>
    </div>
  );
}

function FrontDoor() {
  return (
    <GlowCard>
      <div className="flex flex-col gap-4">
        <div className="flex items-start gap-3">
          <div className="grid h-10 w-10 shrink-0 place-items-center rounded-full bg-[var(--ox-tint)] text-ox">
            <DoorOpen size={20} />
          </div>
          <div className="min-w-0">
            <div className="rail-group-label mb-1">one front door</div>
            <p className="font-title text-subhead text-ink">One connection in your agent, many capabilities behind it</p>
            <p className="mt-1 text-label text-muted-foreground">
              Add the harness once. It exposes its own namespaced verbs and brokers every other MCP server you register
              through the same endpoint, so your agent only ever holds one connection.
            </p>
          </div>
        </div>

        <Tabs defaultValue="claude">
          <TabsList>
            {CLIENTS.map((c) => (
              <TabsTrigger key={c.kind} value={c.kind}>
                {c.label}
              </TabsTrigger>
            ))}
          </TabsList>
          {CLIENTS.map((c) => (
            <TabsContent key={c.kind} value={c.kind} className="mt-3">
              <SnippetBlock client={c.kind} />
            </TabsContent>
          ))}
        </Tabs>
      </div>
    </GlowCard>
  );
}

// --- Capability namespaces -------------------------------------------------

function NamespaceRow({
  ns,
  onToggle,
  pending,
}: {
  ns: CapabilityNamespace;
  onToggle: (id: string, enabled: boolean) => void;
  pending: boolean;
}) {
  const switchId = `ns-${ns.id}`;
  return (
    <div
      className={cn(
        "flex items-center justify-between gap-4 px-4 py-3 transition-opacity",
        !ns.enabled && "opacity-60",
      )}
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <Label htmlFor={switchId} className="cursor-pointer text-body text-ink">
            {ns.label}
          </Label>
          <Badge tone={ns.enabled ? "accent" : "neutral"}>{ns.verbs} verbs</Badge>
        </div>
        <p className="mt-0.5 text-label text-muted-foreground">{ns.description}</p>
      </div>
      <Switch
        id={switchId}
        checked={ns.enabled}
        disabled={pending}
        onCheckedChange={(v) => onToggle(ns.id, v)}
        aria-label={`Toggle ${ns.label} namespace`}
      />
    </div>
  );
}

// --- Brokered servers ------------------------------------------------------

const BROKER_STATUS: Record<
  BrokeredServer["status"],
  { dot: "live" | "error" | "idle"; tone: BadgeProps["tone"]; label: string }
> = {
  connected: { dot: "live", tone: "live", label: "connected" },
  error: { dot: "error", tone: "warn", label: "error" },
  disabled: { dot: "idle", tone: "neutral", label: "disabled" },
};

function BrokerRow({ server, onRemove }: { server: BrokeredServer; onRemove: (id: string) => void }) {
  const s = BROKER_STATUS[server.status];
  return (
    <div className="flex items-center justify-between gap-4 px-4 py-3">
      <div className="flex min-w-0 items-center gap-3">
        <Server size={15} className="shrink-0 text-faint" />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate font-mono text-label text-ink">{server.name}</span>
            <Badge tone="neutral">{server.transport}</Badge>
          </div>
          <div className="mt-0.5 truncate font-mono text-[11px] text-faint">{server.url ?? "(local stdio)"}</div>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <span className="hidden font-mono text-[11px] text-faint sm:inline">{server.tools} tools</span>
        <StatusDot status={s.dot} pulse={server.status === "connected"} />
        <Badge tone={s.tone}>{s.label}</Badge>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 text-muted-foreground hover:text-ox"
          onClick={() => onRemove(server.id)}
          aria-label={`Remove ${server.name}`}
        >
          <Trash2 size={14} />
        </Button>
      </div>
    </div>
  );
}

function RegisterServerDialog({
  onRegister,
}: {
  onRegister: (server: { name: string; transport: "http" | "stdio"; url?: string }) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const [name, setName] = React.useState("");
  const [transport, setTransport] = React.useState<"http" | "stdio">("http");
  const [url, setUrl] = React.useState("");

  const urlRequired = transport === "http";
  const valid = name.trim().length > 0 && (!urlRequired || url.trim().length > 0);

  function reset() {
    setName("");
    setTransport("http");
    setUrl("");
  }

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!valid) return;
    onRegister({
      name: name.trim(),
      transport,
      url: urlRequired ? url.trim() : url.trim() || undefined,
    });
    reset();
    setOpen(false);
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
        <Button variant="primary" size="sm">
          <Plus size={14} /> Register MCP server
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Register MCP server</DialogTitle>
          <DialogDescription>
            Broker an external MCP server behind the harness. Its tools become reachable through the one harness
            connection your agent already holds.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={submit} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="broker-name">Name</Label>
            <Input
              id="broker-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. firecrawl"
              autoFocus
            />
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="broker-transport">Transport</Label>
            <Select value={transport} onValueChange={(v) => setTransport(v as "http" | "stdio")}>
              <SelectTrigger id="broker-transport" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="http">http</SelectItem>
                <SelectItem value="stdio">stdio</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="broker-url">{urlRequired ? "URL" : "Command (optional)"}</Label>
            <Input
              id="broker-url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder={urlRequired ? "https://mcp.example.com" : "npx -y some-mcp-server"}
            />
          </div>

          <div className="flex items-center justify-end gap-2 pt-2">
            <DialogClose asChild>
              <Button type="button" variant="ghost" size="sm">
                Cancel
              </Button>
            </DialogClose>
            <Button type="submit" variant="primary" size="sm" disabled={!valid}>
              <Plus size={14} /> Register
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// --- The hub ---------------------------------------------------------------

export function McpHub() {
  const { data, loading, error, reload } = useAsync(() => harness.getMcpHub(), []);
  // Optimistic local copy so toggles + registrations reflect immediately. The
  // namespace toggle persists through harness.toggleNamespace; brokered-server
  // edits are session-local (no client verb), so we keep them in this state.
  // We seed local state during render when a fresh fetch arrives (the React
  // "adjusting state when a prop changes" pattern) instead of via an effect.
  const [hub, setHub] = React.useState<McpHubState | null>(null);
  const [seededFrom, setSeededFrom] = React.useState<McpHubState | null>(null);
  const [pendingNs, setPendingNs] = React.useState<string | null>(null);

  if (data && data !== seededFrom) {
    setSeededFrom(data);
    setHub({
      namespaces: data.namespaces.map((n) => ({ ...n })),
      brokered: data.brokered.map((b) => ({ ...b })),
    });
  }

  const toggleNamespace = React.useCallback(
    async (id: string, enabled: boolean) => {
      setPendingNs(id);
      // Reflect the toggle immediately.
      setHub((prev) =>
        prev ? { ...prev, namespaces: prev.namespaces.map((n) => (n.id === id ? { ...n, enabled } : n)) } : prev,
      );
      try {
        await harness.toggleNamespace(id, enabled);
        const ns = hub?.namespaces.find((n) => n.id === id);
        toast.success(`${ns?.label ?? "namespace"} ${enabled ? "exposed" : "hidden"}`);
      } catch (e) {
        // Revert on failure.
        setHub((prev) =>
          prev
            ? { ...prev, namespaces: prev.namespaces.map((n) => (n.id === id ? { ...n, enabled: !enabled } : n)) }
            : prev,
        );
        toast.error(e instanceof Error ? e.message : "Could not toggle namespace");
      } finally {
        setPendingNs(null);
      }
    },
    [hub],
  );

  const registerServer = React.useCallback(
    (input: { name: string; transport: "http" | "stdio"; url?: string }) => {
      setHub((prev) => {
        if (!prev) return prev;
        const server: BrokeredServer = {
          id: `b_${Date.now()}`,
          name: input.name,
          transport: input.transport,
          url: input.url,
          status: "connected",
          tools: 0,
        };
        return { ...prev, brokered: [...prev.brokered, server] };
      });
      toast.success(`${input.name} brokered behind the harness`);
    },
    [],
  );

  const removeServer = React.useCallback((id: string) => {
    setHub((prev) => {
      if (!prev) return prev;
      const target = prev.brokered.find((b) => b.id === id);
      if (target) toast.success(`${target.name} removed`);
      return { ...prev, brokered: prev.brokered.filter((b) => b.id !== id) };
    });
  }, []);

  const exposedVerbs = React.useMemo(
    () => (hub ? hub.namespaces.filter((n) => n.enabled).reduce((sum, n) => sum + n.verbs, 0) : 0),
    [hub],
  );

  return (
    <RetroFrame className="p-5">
      <div className="space-y-6">
        {/* Hero: the one front door. */}
        <FrontDoor />

        {loading || !hub ? (
          <div className="grid gap-4 lg:grid-cols-2">
            <Skeleton className="h-64 w-full" />
            <Skeleton className="h-64 w-full" />
          </div>
        ) : error ? (
          <EmptyState
            icon={AlertTriangle}
            title="Could not load the MCP hub"
            description={error}
            action={
              <Button variant="outline" size="sm" onClick={reload}>
                <RefreshCw size={14} /> Retry
              </Button>
            }
          />
        ) : (
          <div className="grid gap-4 lg:grid-cols-2">
            {/* Capability namespaces. */}
            <Card calm className="overflow-hidden">
              <CardHeader className="flex-row items-center justify-between gap-3 space-y-0">
                <div className="flex items-center gap-2">
                  <Layers size={16} className="text-muted-foreground" />
                  <CardTitle className="text-body">Capabilities</CardTitle>
                </div>
                <Badge tone="accent">{exposedVerbs} verbs exposed</Badge>
              </CardHeader>
              <CardDescription className="px-4 pb-2">
                Toggle which of the harness&apos;s own namespaces it exposes through the front door.
              </CardDescription>
              <CardContent className="px-0 pb-0">
                <div className="divide-y divide-line border-t border-line">
                  {hub.namespaces.map((ns) => (
                    <NamespaceRow
                      key={ns.id}
                      ns={ns}
                      onToggle={toggleNamespace}
                      pending={pendingNs === ns.id}
                    />
                  ))}
                </div>
              </CardContent>
            </Card>

            {/* Brokered servers. */}
            <Card calm className="overflow-hidden">
              <CardHeader className="flex-row items-center justify-between gap-3 space-y-0">
                <div className="flex items-center gap-2">
                  <Plug size={16} className="text-muted-foreground" />
                  <CardTitle className="text-body">Brokered servers</CardTitle>
                </div>
                <RegisterServerDialog onRegister={registerServer} />
              </CardHeader>
              <CardDescription className="px-4 pb-2">
                External MCP servers reachable through the same harness connection.
              </CardDescription>
              <CardContent className="px-0 pb-0">
                {hub.brokered.length === 0 ? (
                  <div className="px-4 pb-4">
                    <EmptyState
                      icon={Server}
                      title="No servers brokered yet"
                      description="Register an MCP server to expose its tools through the harness."
                    />
                  </div>
                ) : (
                  <div className="divide-y divide-line border-t border-line">
                    {hub.brokered.map((server) => (
                      <BrokerRow key={server.id} server={server} onRemove={removeServer} />
                    ))}
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        )}

        <Separator />
        <p className="font-mono text-[11px] text-faint">
          The key is baked into the snippet server-side. Replace{" "}
          <span className="text-muted-foreground">{PLACEHOLDER_KEY}</span> with a key from the Keys page.
        </p>
      </div>
    </RetroFrame>
  );
}
