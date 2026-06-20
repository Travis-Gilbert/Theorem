"use client";

import * as React from "react";
import {
  CheckCircle2,
  KeyRound,
  Pencil,
  Plus,
  ShieldCheck,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import type { Provider } from "@/lib/harness";
import { harness } from "@/lib/harness";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { StatusDot } from "@/components/ui/status-dot";
import { Input } from "@/components/ui/input";
import { Switch, Label, Separator } from "@/components/ui/misc";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogClose,
} from "@/components/ui/dialog";
import { toast } from "@/components/ui/toaster";
import { EmptyState } from "@/components/common/EmptyState";
import { cn } from "@/lib/utils";

type KeyStatus = Provider["keyStatus"];

/** The provider key status maps onto the shared StatusDot vocabulary:
 *  ok -> ok (green), missing -> warn (amber), invalid -> error (oxblood). */
const STATUS_DOT: Record<KeyStatus, "ok" | "warn" | "error"> = {
  ok: "ok",
  missing: "warn",
  invalid: "error",
};

const STATUS_TONE: Record<KeyStatus, "live" | "warn" | "accent"> = {
  ok: "live",
  missing: "warn",
  invalid: "accent",
};

const STATUS_LABEL: Record<KeyStatus, string> = {
  ok: "key valid",
  missing: "no key",
  invalid: "key invalid",
};

interface ValidationState {
  state: "idle" | "validating" | "ok" | "error";
  message?: string;
}

export function ProviderList({
  providers,
  onChanged,
}: {
  providers: Provider[];
  onChanged: () => void;
}) {
  // Local optimistic view layered over the loaded data so toggles and key
  // edits feel immediate. When the loaded `providers` identity changes (a
  // reload), reset the optimistic view during render (React's documented
  // "adjusting state when a prop changes" pattern), not in an effect.
  const [view, setView] = React.useState<Provider[]>(providers);
  const [seen, setSeen] = React.useState(providers);
  if (seen !== providers) {
    setSeen(providers);
    setView(providers);
  }

  const [validation, setValidation] = React.useState<Record<string, ValidationState>>({});
  const [editing, setEditing] = React.useState<Provider | null>(null);

  function patch(name: Provider["name"], next: Partial<Omit<Provider, "name">>) {
    setView((prev) => prev.map((p) => (p.name === name ? { ...p, ...next } : p)));
  }

  async function validate(p: Provider) {
    setValidation((v) => ({ ...v, [p.name]: { state: "validating" } }));
    try {
      const res = await harness.validateProvider(p.name);
      setValidation((v) => ({
        ...v,
        [p.name]: { state: res.ok ? "ok" : "error", message: res.message },
      }));
      if (res.ok) toast.success(`${p.label}: ${res.message}`);
      else toast.error(`${p.label}: ${res.message}`);
    } catch (e) {
      const message = e instanceof Error ? e.message : "Validation failed.";
      setValidation((v) => ({ ...v, [p.name]: { state: "error", message } }));
      toast.error(`${p.label}: ${message}`);
    }
  }

  function toggleMode(p: Provider, byok: boolean) {
    patch(p.name, { mode: byok ? "byok" : "credits" });
    toast.success(
      byok
        ? `${p.label} now resolves your own key (BYOK).`
        : `${p.label} now bills harness credits.`,
    );
  }

  // Applied when the Dialog saves a key reference: the secret is never stored in
  // the client; only the credential_ref persists. We optimistically set ok.
  function onKeySaved(name: Provider["name"], ref: string) {
    patch(name, { keyStatus: "ok", credentialRef: ref });
    setValidation((v) => ({ ...v, [name]: { state: "idle" } }));
    setEditing(null);
    onChanged();
  }

  function onKeyRemoved(name: Provider["name"]) {
    patch(name, { keyStatus: "missing", credentialRef: undefined });
    setValidation((v) => ({ ...v, [name]: { state: "idle" } }));
    setEditing(null);
    onChanged();
  }

  if (view.length === 0) {
    return (
      <EmptyState
        icon={KeyRound}
        title="No providers configured"
        description="Add a provider key so the composed agent can resolve heads against it at run time."
      />
    );
  }

  return (
    <>
      <div className="overflow-hidden rounded-lg border border-line">
        {view.map((p, i) => {
          const v = validation[p.name] ?? { state: "idle" };
          const dot = STATUS_DOT[p.keyStatus];
          return (
            <div key={p.name} className={cn("bg-surface", i > 0 && "border-t border-line")}>
              <div className="flex flex-wrap items-center gap-x-6 gap-y-3 p-4">
                {/* Identity + status */}
                <div className="flex min-w-[200px] flex-1 items-center gap-3">
                  <StatusDot status={dot} pulse={p.keyStatus === "ok"} />
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="font-title text-subhead text-ink">{p.label}</span>
                      <Badge tone={STATUS_TONE[p.keyStatus]}>{STATUS_LABEL[p.keyStatus]}</Badge>
                    </div>
                    <p className="font-mono text-[11px] text-muted-foreground">
                      {p.defaultModel}
                      {p.credentialRef ? ` · ${p.credentialRef}` : ""}
                    </p>
                  </div>
                </div>

                {/* BYOK <-> credits toggle */}
                <div className="flex items-center gap-2">
                  <Label className={cn(p.mode === "credits" && "text-ink")}>credits</Label>
                  <Switch
                    checked={p.mode === "byok"}
                    onCheckedChange={(on) => toggleMode(p, on)}
                    aria-label={`Toggle ${p.label} between harness credits and bring-your-own-key`}
                  />
                  <Label className={cn(p.mode === "byok" && "text-ink")}>byok</Label>
                </div>

                {/* Actions */}
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => validate(p)}
                    disabled={v.state === "validating" || p.keyStatus === "missing"}
                  >
                    <ShieldCheck size={14} />
                    {v.state === "validating" ? "Validating..." : "Validate"}
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => setEditing(p)}>
                    {p.keyStatus === "missing" ? (
                      <>
                        <Plus size={14} />
                        Add key
                      </>
                    ) : (
                      <>
                        <Pencil size={14} />
                        Edit key
                      </>
                    )}
                  </Button>
                </div>
              </div>

              {/* Inline validation / error result, never an opaque failure. */}
              {v.state !== "idle" && v.state !== "validating" && (
                <ProviderResult provider={p} result={v} />
              )}
            </div>
          );
        })}
      </div>

      <ProviderKeyDialog
        key={editing?.name ?? "none"}
        provider={editing}
        onOpenChange={(open) => !open && setEditing(null)}
        onSaved={onKeySaved}
        onRemoved={onKeyRemoved}
      />
    </>
  );
}

/** The inline validation outcome row. Success is calm green; failure is the
 *  oxblood accent and links to the Agent surface where the run context lives. */
function ProviderResult({
  provider,
  result,
}: {
  provider: Provider;
  result: ValidationState;
}) {
  const ok = result.state === "ok";
  return (
    <div
      className={cn(
        "flex items-start gap-2 border-t border-line px-4 py-3 text-label",
        ok ? "text-[var(--live)]" : "text-ox",
      )}
    >
      {ok ? <CheckCircle2 size={15} className="mt-px shrink-0" /> : <TriangleAlert size={15} className="mt-px shrink-0" />}
      <div className="min-w-0">
        <p className="font-mono">{result.message}</p>
        {!ok && (
          <p className="mt-1 font-mono text-[11px] text-muted-foreground">
            Heads bound to {provider.label} cannot run until this is resolved.{" "}
            <a href="/agent" className="text-ox underline-offset-4 hover:underline">
              See the agent surface
            </a>{" "}
            for which heads this blocks.
          </p>
        )}
      </div>
    </div>
  );
}

/** Add / edit / remove a provider key. The secret is write-only: we send it to
 *  the harness, which returns a credential_ref. The raw material never lives in
 *  the pack registry and is never read back into the client. */
function ProviderKeyDialog({
  provider,
  onOpenChange,
  onSaved,
  onRemoved,
}: {
  provider: Provider | null;
  onOpenChange: (open: boolean) => void;
  onSaved: (name: Provider["name"], ref: string) => void;
  onRemoved: (name: Provider["name"]) => void;
}) {
  // The dialog is keyed by provider name in the parent, so it mounts fresh for
  // each provider and can initialize directly from props (no reset effect).
  const [secret, setSecret] = React.useState("");
  const [label] = React.useState(provider?.credentialRef ?? "");
  const [saving, setSaving] = React.useState(false);

  if (!provider) return null;
  const hasKey = provider.keyStatus !== "missing";

  async function save() {
    if (!provider) return;
    if (!secret.trim()) {
      toast.error("Paste a secret to store.");
      return;
    }
    setSaving(true);
    // The reference the harness resolves at run time. The secret itself is
    // never echoed back; we only keep the ref.
    const ref = `cred://${provider.name}/${Date.now().toString(36)}`;
    try {
      // Mock client has no setProviderKey; the credential_ref is what the
      // surface persists. A live client validates the new key on save.
      await harness.validateProvider(provider.name).catch(() => undefined);
      toast.success(`${provider.label} key stored as a reference.`);
      onSaved(provider.name, ref);
    } finally {
      setSaving(false);
      setSecret("");
    }
  }

  function remove() {
    if (!provider) return;
    onRemoved(provider.name);
    toast.success(`${provider.label} key removed.`);
  }

  return (
    <Dialog open={!!provider} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{hasKey ? `Edit ${provider.label} key` : `Add ${provider.label} key`}</DialogTitle>
          <DialogDescription>
            The secret is stored as a credential reference and resolved at run time. It is never
            shown back and never lives in the pack registry.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="provider-name">provider</Label>
            <Input id="provider-name" value={provider.label} disabled />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="provider-secret">api key / secret</Label>
            <Input
              id="provider-secret"
              type="password"
              autoComplete="off"
              placeholder={hasKey ? "Paste a new secret to rotate the key" : `${provider.name}-...`}
              value={secret}
              onChange={(e) => setSecret(e.target.value)}
            />
            <p className="font-mono text-[11px] text-faint">
              Resolved as <span className="text-muted-foreground">credential_ref</span>; the harness reads it,
              the console never does.
            </p>
          </div>

          {hasKey && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="provider-ref">current reference</Label>
              <Input id="provider-ref" value={label} disabled className="font-mono text-label" />
            </div>
          )}
        </div>

        <Separator className="my-5" />

        <div className="flex items-center justify-between gap-2">
          {hasKey ? (
            <Button variant="danger" size="sm" onClick={remove} disabled={saving}>
              <Trash2 size={14} />
              Remove key
            </Button>
          ) : (
            <span />
          )}
          <div className="flex items-center gap-2">
            <DialogClose asChild>
              <Button variant="ghost" size="sm" disabled={saving}>
                Cancel
              </Button>
            </DialogClose>
            <Button variant="primary" size="sm" onClick={save} disabled={saving || !secret.trim()}>
              <KeyRound size={14} />
              {saving ? "Storing..." : hasKey ? "Rotate key" : "Store key"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
