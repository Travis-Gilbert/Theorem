"use client";

import * as React from "react";
import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import Link from "next/link";
import { MessageSquare, AlertTriangle, KeyRound, Sparkles } from "lucide-react";
import { harness, type ChatMessage, type AgentBinding } from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";
import { usePageToc } from "@/components/island/useScrollSpy";
import { PageHeader, Section } from "@/components/common/PageHeader";
import { EmptyState } from "@/components/common/EmptyState";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/misc";
import { Card, CardContent } from "@/components/ui/card";
import { toast } from "@/components/ui/toaster";
import { Thread } from "@/components/agent/Thread";
import { Composer, type Attachment } from "@/components/agent/Composer";
import { HeadPanel, HeadPanelSkeleton } from "@/components/agent/HeadPanel";

/** A new user turn, locally minted before the assistant turn streams back. */
function userMessage(content: string): ChatMessage {
  return { id: `u_${Date.now()}`, role: "user", content, at: new Date().toISOString() };
}

/** Placeholder assistant turn shown while the run is in flight. */
function pendingMessage(): ChatMessage {
  return {
    id: `a_${Date.now()}`,
    role: "assistant",
    content: "",
    at: new Date().toISOString(),
    verdict: "pending",
    trace: [],
  };
}

function AgentSurface() {
  const search = useSearchParams();
  const { data: binding, loading, error, reload } = useAsync<AgentBinding>(() => harness.getBinding(), []);

  const [messages, setMessages] = React.useState<ChatMessage[]>([]);
  // Prefill from the omnibar "Ask the Theorem agent" handoff (?prompt=). Read
  // once via a lazy initializer so it lands as the first paint, no effect.
  const [draft, setDraft] = React.useState(() => search.get("prompt") ?? "");
  const [attachments, setAttachments] = React.useState<Attachment[]>([]);
  const [streamingId, setStreamingId] = React.useState<string | null>(null);
  const sending = streamingId !== null;

  const hasKeyedHead = !!binding && binding.heads.some((h) => h.keyStatus === "ok");

  const addAttachment = React.useCallback((a: Attachment) => {
    setAttachments((prev) => (prev.some((x) => x.ref === a.ref) ? prev : [...prev, a]));
  }, []);
  const removeAttachment = React.useCallback((ref: string) => {
    setAttachments((prev) => prev.filter((a) => a.ref !== ref));
  }, []);

  const onSend = React.useCallback(async () => {
    const prompt = draft.trim();
    if (!prompt || sending || !binding) return;

    const turn = userMessage(prompt);
    const pending = pendingMessage();
    setMessages((prev) => [...prev, turn, pending]);
    setStreamingId(pending.id);
    setDraft("");

    // Attachments + the binding memory scopes become the run scope.
    const scope = [...attachments.map((a) => a.ref), ...binding.scope.memoryScopes];
    const usedAttachments = attachments;
    setAttachments([]);

    try {
      const result = await harness.runAgent(prompt, scope);
      // Splice the real assistant turn in over the placeholder, keeping its id
      // so the streaming indicator hands off cleanly.
      setMessages((prev) =>
        prev.map((m) => (m.id === pending.id ? { ...result, id: pending.id } : m)),
      );
      if (result.verdict === "flagged" || result.verdict === "blocked") {
        toast(`Alignment gate: ${result.verdict}`);
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const invalidKey = /key|provider|auth|401|403|credential/i.test(msg);
      setMessages((prev) =>
        prev.map((m) =>
          m.id === pending.id
            ? {
                ...m,
                content: invalidKey
                  ? `The run failed on a provider key: ${msg}`
                  : `The run errored: ${msg}`,
                verdict: "blocked",
                trace: [
                  {
                    id: `err_${Date.now()}`,
                    role: "system",
                    content: invalidKey ? "alignment-gate: blocked (provider key)" : "alignment-gate: blocked (error)",
                    at: new Date().toISOString(),
                  },
                ],
              }
            : m,
        ),
      );
      toast(invalidKey ? "Invalid provider key. Add it in Providers." : "Agent run errored.");
      setAttachments(usedAttachments); // give the context back so the turn can be retried
    } finally {
      setStreamingId(null);
    }
  }, [draft, sending, binding, attachments]);

  // --- Loading ------------------------------------------------------------
  if (loading) {
    return (
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_320px]">
        <div className="space-y-3">
          <Skeleton className="h-40 w-full" />
          <Skeleton className="h-28 w-full" />
        </div>
        <HeadPanelSkeleton />
      </div>
    );
  }

  // --- Error --------------------------------------------------------------
  if (error || !binding) {
    return (
      <EmptyState
        icon={AlertTriangle}
        title="Could not load the agent binding"
        description={error ?? "The composed agent binding (agent:theorem) was unavailable."}
        action={
          <Button variant="outline" size="sm" onClick={reload}>
            Retry
          </Button>
        }
      />
    );
  }

  // --- Loaded -------------------------------------------------------------
  return (
    <div className="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_320px]">
      {/* main: thread + composer */}
      <div className="flex min-w-0 flex-col">
        <Section id="conversation" title="Conversation" className="mb-4 flex-1">
          {messages.length === 0 ? (
            !hasKeyedHead ? (
              <EmptyState
                icon={KeyRound}
                title="Add a provider key to begin"
                description="The composed agent runs frontier heads as peers. None of them has a usable provider key yet, so a turn cannot run. Add a key and the composer wakes up."
                action={
                  <Button asChild variant="primary" size="sm">
                    <Link href="/providers">
                      <KeyRound size={13} /> Go to Providers
                    </Link>
                  </Button>
                }
              />
            ) : (
              <Card calm>
                <CardContent className="flex flex-col items-center gap-2 p-8 text-center">
                  <span className="grid h-11 w-11 place-items-center rounded-full bg-surface-2 text-muted-foreground">
                    <Sparkles size={20} />
                  </span>
                  <p className="font-title text-subhead text-ink">Task the Theorem agent</p>
                  <p className="max-w-md text-label text-muted-foreground">
                    Send a prompt and the heads run as peers over your memory scopes and rooms.
                    Attach an atom or a room to ground the turn. The alignment gate&rsquo;s verdict
                    shows when the run closes.
                  </p>
                </CardContent>
              </Card>
            )
          ) : (
            <Thread messages={messages} streamingId={streamingId} />
          )}
        </Section>

        <div className="sticky bottom-4">
          <Composer
            value={draft}
            onChange={setDraft}
            attachments={attachments}
            onAttach={addAttachment}
            onRemoveAttachment={removeAttachment}
            onSend={onSend}
            sending={sending}
            disabled={!hasKeyedHead}
          />
        </div>
      </div>

      {/* aside: composed-agent makeup + scope + live run trace */}
      <aside className="min-w-0 lg:sticky lg:top-2 lg:self-start">
        <Section id="makeup" title="Makeup" depth={2}>
          <HeadPanel binding={binding} />
        </Section>
      </aside>
    </div>
  );
}

export default function AgentPage() {
  usePageToc();
  return (
    <>
      <PageHeader
        eyebrow="agent:theorem"
        title="Agent"
        description="Converse with and task the composed Theorem agent. Multiple frontier heads run as peers over the shared substrate."
        actions={
          <Button asChild variant="outline" size="sm">
            <Link href="/runs">
              <MessageSquare size={13} /> Run ledger
            </Link>
          </Button>
        }
      />
      <Suspense
        fallback={
          <div className="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_320px]">
            <Skeleton className="h-64 w-full" />
            <HeadPanelSkeleton />
          </div>
        }
      >
        <AgentSurface />
      </Suspense>
    </>
  );
}
