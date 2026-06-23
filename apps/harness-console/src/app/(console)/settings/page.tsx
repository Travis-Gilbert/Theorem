"use client";

import * as React from "react";
import { Check, Moon, Plus, Sun, Users } from "lucide-react";
import { usePageToc } from "@/components/island/useScrollSpy";
import { PageHeader, Section } from "@/components/common/PageHeader";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label, Separator } from "@/components/ui/misc";
import { toast } from "@/components/ui/toaster";
import { cn } from "@/lib/utils";

type Theme = "light" | "dark";

const THEME_KEY = "harness-console-theme";

/** The scopes a key gets by default when minted from this tenant. The full set
 *  mirrors the harness scope vocabulary; the toggled ones become the default. */
const ALL_SCOPES = [
  "memory:read",
  "memory:write",
  "coordination:read",
  "coordination:write",
  "run:read",
  "run:write",
  "skill:read",
  "skill:write",
] as const;

const DEFAULT_SCOPES = ["memory:read", "memory:write", "coordination:read", "run:write"];

/** Resolve the starting theme on the client. SSR returns the default so the
 *  first paint matches; the effect re-syncs <html> after hydration. */
function readInitialTheme(): Theme {
  if (typeof document === "undefined") return "light";
  const live = document.documentElement.dataset.theme as Theme | undefined;
  if (live === "light" || live === "dark") return live;
  let stored: string | null = null;
  try {
    stored = window.localStorage.getItem(THEME_KEY);
  } catch {
    stored = null;
  }
  return stored === "dark" ? "dark" : "light";
}

export default function SettingsPage() {
  usePageToc();

  const [tenant, setTenant] = React.useState(process.env.NEXT_PUBLIC_DEFAULT_TENANT ?? "default");
  const [scopes, setScopes] = React.useState<string[]>(DEFAULT_SCOPES);

  // Read the persisted theme lazily in the initializer (runs once, client-only):
  // the live <html data-theme> may already be set by an earlier visit, so
  // reflect whichever is present, falling back to localStorage then light.
  const [theme, setTheme] = React.useState<Theme>(() => readInitialTheme());

  // Mirror the chosen theme onto <html> (an external system) whenever it
  // changes. This is the legitimate effect: synchronize React state outward.
  React.useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  function applyTheme(next: Theme) {
    setTheme(next);
    try {
      window.localStorage.setItem(THEME_KEY, next);
    } catch {
      // localStorage may be unavailable (private mode); the dataset still applies.
    }
  }

  function toggleScope(scope: string) {
    setScopes((prev) => (prev.includes(scope) ? prev.filter((s) => s !== scope) : [...prev, scope]));
  }

  function saveTenant() {
    if (!tenant.trim()) {
      toast.error("Tenant name cannot be empty.");
      return;
    }
    toast.success("Tenant settings saved.");
  }

  return (
    <div className="mx-auto max-w-3xl">
      <PageHeader
        eyebrow="Tenant"
        title="Settings"
        description="Your tenant name, the default scopes minted keys receive, and the console theme."
      />

      <Section id="tenant" title="Tenant">
        <Card calm>
          <CardContent className="flex flex-col gap-4 pt-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="tenant-name">tenant name</Label>
              <Input
                id="tenant-name"
                value={tenant}
                onChange={(e) => setTenant(e.target.value)}
                placeholder="your-tenant"
              />
              <p className="font-mono text-[11px] text-faint">
                Baked into every key and resolved server side. All memory, rooms, and runs are scoped
                to it.
              </p>
            </div>
            <div>
              <Button variant="primary" size="sm" onClick={saveTenant}>
                Save tenant
              </Button>
            </div>
          </CardContent>
        </Card>
      </Section>

      <Section id="default-scopes" title="Default scopes">
        <Card calm>
          <CardContent className="pt-4">
            <p className="mb-3 text-label text-muted-foreground">
              The scopes a new key receives unless narrowed at creation. Toggle a chip to include or
              exclude it.
            </p>
            <div className="flex flex-wrap gap-2">
              {ALL_SCOPES.map((scope) => {
                const on = scopes.includes(scope);
                return (
                  <button
                    key={scope}
                    type="button"
                    onClick={() => toggleScope(scope)}
                    aria-pressed={on}
                    className={cn(
                      "inline-flex items-center gap-1.5 rounded-full border px-3 py-1 font-mono text-label transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--ox-ring)]",
                      on
                        ? "border-[var(--ox)] bg-[var(--ox-tint)] text-ox"
                        : "border-line bg-bg text-muted-foreground hover:bg-surface-2",
                    )}
                  >
                    {on ? <Check size={12} /> : <Plus size={12} />}
                    {scope}
                  </button>
                );
              })}
            </div>
            <p className="mt-4 border-t border-line pt-3 font-mono text-[11px] text-faint">
              {scopes.length} of {ALL_SCOPES.length} scopes selected
              {scopes.length === 0 && " — keys would be minted read-nothing"}
            </p>
          </CardContent>
        </Card>
      </Section>

      <Section id="theme" title="Theme">
        <Card calm>
          <CardContent className="pt-4">
            <p className="mb-3 text-label text-muted-foreground">
              Light is the default paper field. Switch to dark for low-light work; the choice is
              remembered on this device.
            </p>
            <div className="flex flex-col gap-2 sm:flex-row">
              <ThemeOption
                active={theme === "light"}
                onClick={() => applyTheme("light")}
                icon={Sun}
                title="Paper (light)"
                desc="White field, grey surfaces, oxblood accent."
              />
              <ThemeOption
                active={theme === "dark"}
                onClick={() => applyTheme("dark")}
                icon={Moon}
                title="Dark"
                desc="Inked field, contrast-held accent and status."
              />
            </div>
          </CardContent>
        </Card>
      </Section>

      <Section id="members" title="Members">
        <Card calm>
          <CardContent className="pt-4">
            <div className="flex items-start justify-between gap-3 rounded-md border border-dashed border-line bg-bg p-4">
              <div className="flex items-start gap-3">
                <div className="grid h-9 w-9 shrink-0 place-items-center rounded-md bg-surface-2 text-muted-foreground">
                  <Users size={16} />
                </div>
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-title text-subhead text-ink">Member management</span>
                    <Badge tone="neutral">for later</Badge>
                  </div>
                  <p className="mt-0.5 font-mono text-[11px] text-muted-foreground">
                    Invite teammates, assign roles, and scope their access. Not yet available.
                  </p>
                </div>
              </div>
              <Button variant="subtle" size="sm" disabled>
                <Plus size={14} />
                Invite
              </Button>
            </div>

            <Separator className="my-4" />

            {/* The single current member, the owner, shown as the read-only state. */}
            <div className="flex items-center justify-between gap-3 px-1">
              <div className="flex items-center gap-2">
                <span className="grid h-7 w-7 place-items-center rounded-full bg-ox text-[11px] font-mono text-white">
                  T
                </span>
                <span className="font-mono text-label text-ink">you</span>
              </div>
              <Badge tone="accent">owner</Badge>
            </div>
          </CardContent>
        </Card>
      </Section>
    </div>
  );
}

function ThemeOption({
  active,
  onClick,
  icon: Icon,
  title,
  desc,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  title: string;
  desc: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "flex flex-1 items-center gap-3 rounded-lg border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--ox-ring)]",
        active ? "border-[var(--ox)] bg-[var(--ox-tint)]" : "border-line bg-bg hover:bg-surface-2",
      )}
    >
      <div
        className={cn(
          "grid h-9 w-9 shrink-0 place-items-center rounded-md",
          active ? "bg-ox text-white" : "bg-surface-2 text-muted-foreground",
        )}
      >
        <Icon size={16} />
      </div>
      <div className="min-w-0">
        <div className="flex items-center gap-1.5">
          <span className="font-mono text-label text-ink">{title}</span>
          {active && <Check size={13} className="text-ox" />}
        </div>
        <div className="font-mono text-[11px] text-muted-foreground">{desc}</div>
      </div>
    </button>
  );
}
