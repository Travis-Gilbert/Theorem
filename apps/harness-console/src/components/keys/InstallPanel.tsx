"use client";

import * as React from "react";
import { Check, Copy, Terminal } from "lucide-react";
import { harness, type ClientKind, type HarnessKey } from "@/lib/harness";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from "@/components/ui/select";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/misc";
import { toast } from "@/components/ui/toaster";
import { cn, keyPrefix } from "@/lib/utils";

/**
 * A copy-to-clipboard code block. Reused for the install snippet and for the
 * one-time secret reveal in the claim flow. Mono, calm surface, a single copy
 * affordance that confirms with a check and a toast.
 */
export function CopyBlock({
  value,
  label,
  toastLabel = "Copied to clipboard",
  className,
}: {
  value: string;
  label?: string;
  toastLabel?: string;
  className?: string;
}) {
  const [copied, setCopied] = React.useState(false);

  const copy = React.useCallback(async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      toast.success(toastLabel);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      toast.error("Could not access the clipboard");
    }
  }, [value, toastLabel]);

  const copyButton = (
    <button
      type="button"
      onClick={copy}
      aria-label="Copy to clipboard"
      className="inline-flex items-center gap-1 rounded border border-line bg-bg px-2 py-1 font-mono text-[11px] text-muted-foreground transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--ox-ring)]"
    >
      {copied ? <Check size={12} className="text-live" /> : <Copy size={12} />}
      {copied ? "Copied" : "Copy"}
    </button>
  );

  return (
    <div className={cn("relative rounded-lg border border-line bg-surface", className)}>
      {label ? (
        <div className="flex items-center justify-between gap-2 border-b border-line px-3 py-2">
          <span className="flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-wide text-muted-foreground">
            <Terminal size={12} /> {label}
          </span>
          {copyButton}
        </div>
      ) : (
        <div className="absolute right-2 top-2">{copyButton}</div>
      )}
      <pre className={cn("overflow-x-auto p-3 font-mono text-[12.5px] leading-relaxed text-ink", !label && "pr-20")}>
        <code>{value}</code>
      </pre>
    </div>
  );
}

const CLIENTS: { value: ClientKind; label: string; blurb: string }[] = [
  { value: "claude", label: "Claude Code", blurb: "Register the harness as an HTTP MCP server in Claude Code." },
  { value: "codex", label: "Codex", blurb: "Add the harness to ~/.codex/config.toml and export the key." },
  { value: "gemini", label: "Gemini", blurb: "Wire the harness into ~/.gemini/settings.json mcpServers." },
  { value: "raw", label: "Raw HTTP", blurb: "Speak MCP JSON-RPC directly over an authenticated POST." },
];

/**
 * The cold-start hero. Pick a client, get the exact copy-paste block. The
 * tenant is baked into the key server side, so the only things the user pastes
 * are the URL and the key. Switching the client regenerates the snippet for the
 * same key, so a connection is always one paste away.
 */
export function InstallPanel({ keyValue }: { keyValue?: HarnessKey | string | null }) {
  const [client, setClient] = React.useState<ClientKind>("claude");

  // Accept a full key value (from a fresh mint / claim) or fall back to a key's
  // displayable prefix. The snippet helper takes whatever string we pass it.
  const token =
    typeof keyValue === "string"
      ? keyValue
      : keyValue?.prefix ?? "hk_live_xxxxxx";

  const isRealKey = typeof keyValue === "string" ? keyValue.length > 0 : Boolean(keyValue);

  const snippet = harness.installSnippet(client, token);
  const active = CLIENTS.find((c) => c.value === client)!;

  return (
    <div className="space-y-4">
      {/* Client picker: Tabs on wide viewports, Select on narrow. */}
      <div className="hidden md:block">
        <Tabs value={client} onValueChange={(v) => setClient(v as ClientKind)}>
          <TabsList className="w-full justify-start">
            {CLIENTS.map((c) => (
              <TabsTrigger key={c.value} value={c.value} className="flex-1">
                {c.label}
              </TabsTrigger>
            ))}
          </TabsList>
          {CLIENTS.map((c) => (
            <TabsContent key={c.value} value={c.value} className="mt-3">
              <SnippetBody blurb={c.blurb} snippet={snippet} token={token} isRealKey={isRealKey} clientLabel={c.label} />
            </TabsContent>
          ))}
        </Tabs>
      </div>

      <div className="md:hidden space-y-3">
        <div className="space-y-1.5">
          <Label htmlFor="client-picker">Client</Label>
          <Select value={client} onValueChange={(v) => setClient(v as ClientKind)}>
            <SelectTrigger id="client-picker" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {CLIENTS.map((c) => (
                <SelectItem key={c.value} value={c.value}>
                  {c.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <SnippetBody blurb={active.blurb} snippet={snippet} token={token} isRealKey={isRealKey} clientLabel={active.label} />
      </div>
    </div>
  );
}

function SnippetBody({
  blurb,
  snippet,
  token,
  isRealKey,
  clientLabel,
}: {
  blurb: string;
  snippet: string;
  token: string;
  isRealKey: boolean;
  clientLabel: string;
}) {
  return (
    <div className="space-y-3">
      <p className="text-label text-muted-foreground">{blurb}</p>
      <CopyBlock value={snippet} label={`${clientLabel} setup`} toastLabel={`${clientLabel} snippet copied`} />
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone="neutral">
          key {keyPrefix(token)}
        </Badge>
        {isRealKey ? (
          <Badge tone="live">tenant baked in</Badge>
        ) : (
          <Badge tone="warn">create a key to fill the snippet</Badge>
        )}
      </div>
      <p className="text-label text-faint">
        Paste this into your shell. The {clientLabel} client connects to the harness over MCP; the tenant is resolved
        from the key, so you never set it by hand. Run a tool (e.g. <span className="font-mono text-muted-foreground">recall</span>)
        to confirm the connection.
      </p>
    </div>
  );
}
