"use client";

import * as React from "react";
import Link from "next/link";
import {
  ArrowRight,
  CheckCircle2,
  Clock,
  Github,
  KeyRound,
  ShieldCheck,
  Sparkles,
  Terminal,
} from "lucide-react";
import { harness, type RegisterResult, type HarnessKey } from "@/lib/harness";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/misc";
import { toast } from "@/components/ui/toaster";
import { CopyBlock, InstallPanel } from "@/components/keys/InstallPanel";
import { relativeTime } from "@/lib/utils";

type Stage = "choose" | "agent" | "signup";

/**
 * The claim flow, modeled on Browser Use: a coding agent provisions its own key
 * before there is an account, and a human claims it later. Lives outside the
 * shell (no rail, no island) as a centered card flow over the ambient field.
 */
export default function ClaimPage() {
  const [stage, setStage] = React.useState<Stage>("choose");

  return (
    <main className="relative z-10 mx-auto flex min-h-dvh w-full max-w-2xl flex-col items-center justify-center gap-8 px-6 py-16">
      <Wordmark />

      {stage === "choose" && <ChooseStage onStage={setStage} />}
      {stage === "agent" && <AgentStage onBack={() => setStage("choose")} />}
      {stage === "signup" && <SignupStage onBack={() => setStage("choose")} />}

      <p className="text-center text-label text-faint">
        Already connected?{" "}
        <Link href="/keys" className="text-ox underline-offset-4 hover:underline">
          Open the console
        </Link>
      </p>
    </main>
  );
}

function Wordmark() {
  return (
    <div className="flex flex-col items-center gap-1.5">
      <span className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.2em] text-muted-foreground">
        <span className="status-dot" style={{ background: "var(--ox)" }} aria-hidden /> Theorems Harness
      </span>
      <h1 className="text-center font-title text-[2rem] leading-tight text-ink">Claim your harness</h1>
      <p className="max-w-md text-center text-label text-muted-foreground">
        Connect a coding agent to a graph-native memory and coordination substrate. Provision a key as the agent, claim
        it as you.
      </p>
    </div>
  );
}

// --- Stage 0: choose a path -------------------------------------------------

function ChooseStage({ onStage }: { onStage: (s: Stage) => void }) {
  return (
    <div className="grid w-full gap-4 sm:grid-cols-2">
      <Card lift className="flex flex-col">
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Terminal size={16} className="text-ox" /> Agent self-registration
          </CardTitle>
          <CardDescription>
            The coding agent mints its own scoped key with no account. You claim it later.
          </CardDescription>
        </CardHeader>
        <CardContent className="mt-auto">
          <Button variant="primary" className="w-full" onClick={() => onStage("agent")}>
            Register as an agent <ArrowRight size={14} />
          </Button>
        </CardContent>
      </Card>

      <Card lift className="flex flex-col">
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Sparkles size={16} className="text-muted-foreground" /> Sign up
          </CardTitle>
          <CardDescription>
            Create an account, get a key instantly, and land on the install panel.
          </CardDescription>
        </CardHeader>
        <CardContent className="mt-auto">
          <Button variant="outline" className="w-full" onClick={() => onStage("signup")}>
            Continue with GitHub <Github size={14} />
          </Button>
        </CardContent>
      </Card>

      <div className="sm:col-span-2">
        <FreeTierNote />
      </div>
    </div>
  );
}

// --- Stage 1: agent self-registration --------------------------------------

function AgentStage({ onBack }: { onBack: () => void }) {
  const [result, setResult] = React.useState<RegisterResult | null>(null);
  const [registering, setRegistering] = React.useState(false);
  const [claimed, setClaimed] = React.useState(false);

  async function register() {
    setRegistering(true);
    try {
      const r = await harness.registerAnonymous();
      setResult(r);
      toast.success("Anonymous key provisioned");
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Registration failed");
    } finally {
      setRegistering(false);
    }
  }

  return (
    <div className="w-full space-y-4">
      {!result ? (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Terminal size={16} className="text-ox" /> Agent self-registration
            </CardTitle>
            <CardDescription>
              Mint an anonymous tenant and a scoped key. No email, no account. The key works immediately; you can claim
              the tenant later to keep it.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <Button variant="primary" className="w-full" onClick={register} disabled={registering}>
              {registering ? "Provisioning..." : "Provision an anonymous key"}
              {!registering && <KeyRound size={14} />}
            </Button>
            <Button variant="ghost" size="sm" className="w-full" onClick={onBack}>
              Back
            </Button>
          </CardContent>
        </Card>
      ) : (
        <>
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <CheckCircle2 size={16} className="text-live" /> Key provisioned
              </CardTitle>
              <CardDescription>
                This is the only time the full key is shown. Copy it now; the harness stores only its prefix.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <CopyBlock value={result.key} label="harness api key" toastLabel="API key copied" />

              <div className="flex flex-wrap items-center gap-2">
                <Badge tone="neutral">tenant {result.tenant}</Badge>
                <Badge tone="warn">
                  <Clock size={11} /> expires {relativeTime(result.expiresAt)}
                </Badge>
              </div>

              <div className="space-y-1.5">
                <p className="font-mono text-[11px] uppercase tracking-wide text-muted-foreground">Claim URL</p>
                <CopyBlock value={result.claimUrl} toastLabel="Claim URL copied" />
                <p className="text-label text-faint">
                  Open this URL signed in to bind the anonymous tenant to your account before it expires.
                </p>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-subhead">Install the connection</CardTitle>
              <CardDescription>Paste this into your client to connect the harness right now.</CardDescription>
            </CardHeader>
            <CardContent>
              <InstallPanel keyValue={result.key} />
            </CardContent>
          </Card>

          <ClaimCard claimed={claimed} onClaim={() => setClaimed(true)} tenant={result.tenant} />
        </>
      )}
    </div>
  );
}

// --- The claim (mock GitHub OAuth) -----------------------------------------

function ClaimCard({ claimed, onClaim, tenant }: { claimed: boolean; onClaim: () => void; tenant: string }) {
  const [claiming, setClaiming] = React.useState(false);

  async function claim() {
    setClaiming(true);
    // Mock OAuth round-trip: in live mode this redirects to GitHub and back.
    await new Promise((r) => setTimeout(r, 600));
    setClaiming(false);
    onClaim();
    toast.success("Tenant claimed");
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Github size={16} className="text-muted-foreground" /> Claim this tenant
        </CardTitle>
        <CardDescription>
          Bind the anonymous tenant <span className="font-mono text-ink">{tenant}</span> to your account so it survives
          past the expiry window.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {claimed ? (
          <div className="flex items-center gap-2 rounded-md border border-[var(--live)] bg-surface px-3 py-2 text-label text-[var(--live)]">
            <CheckCircle2 size={15} />
            <span>
              Claimed and bound to your GitHub account. The key keeps working;{" "}
              <Link href="/keys" className="underline underline-offset-4">
                manage it in the console
              </Link>
              .
            </span>
          </div>
        ) : (
          <Button variant="outline" className="w-full" onClick={claim} disabled={claiming}>
            {claiming ? "Connecting to GitHub..." : "Continue with GitHub"}
            {!claiming && <Github size={14} />}
          </Button>
        )}
      </CardContent>
    </Card>
  );
}

// --- Stage 2: UI signup path -----------------------------------------------

function SignupStage({ onBack }: { onBack: () => void }) {
  const [key, setKey] = React.useState<HarnessKey | null>(null);
  const [signingUp, setSigningUp] = React.useState(false);

  async function signup() {
    setSigningUp(true);
    try {
      // Mock GitHub OAuth, then mint a key instantly so the user lands on the
      // install panel with a working key.
      await new Promise((r) => setTimeout(r, 600));
      const k = await harness.createKey("Default (web signup)", [
        "memory:read",
        "memory:write",
        "coordination:read",
        "run:write",
      ]);
      setKey(k);
      toast.success("Account created, key minted");
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Signup failed");
    } finally {
      setSigningUp(false);
    }
  }

  return (
    <div className="w-full space-y-4">
      {!key ? (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Sparkles size={16} className="text-ox" /> Sign up
            </CardTitle>
            <CardDescription>
              Continue with GitHub. We mint a default key instantly and drop you on the install panel.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <Button variant="primary" className="w-full" onClick={signup} disabled={signingUp}>
              {signingUp ? "Creating your account..." : "Continue with GitHub"}
              {!signingUp && <Github size={14} />}
            </Button>
            <Button variant="ghost" size="sm" className="w-full" onClick={onBack}>
              Back
            </Button>
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <CheckCircle2 size={16} className="text-live" /> You're in
            </CardTitle>
            <CardDescription>
              A key named <span className="font-mono text-ink">{key.name}</span> is ready. Paste the block into your
              client to connect.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <InstallPanel keyValue={key} />
            <Separator />
            <Link href="/keys">
              <Button variant="outline" size="sm" className="w-full">
                Open the console <ArrowRight size={14} />
              </Button>
            </Link>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

// --- Free tier note ---------------------------------------------------------

function FreeTierNote() {
  return (
    <Card calm>
      <CardContent className="flex flex-col gap-2 pt-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-start gap-2.5">
          <ShieldCheck size={16} className="mt-0.5 shrink-0 text-muted-foreground" />
          <div className="text-label text-muted-foreground">
            <p className="font-title text-body text-ink">Free tier, honest limits</p>
            <p className="mt-0.5">
              10,000 requests / period, shared compute, memory + coordination + run scopes. No card. Anonymous keys
              expire in 24h unless claimed.
            </p>
          </div>
        </div>
        <Badge tone="neutral" className="shrink-0">
          Free
        </Badge>
      </CardContent>
    </Card>
  );
}
