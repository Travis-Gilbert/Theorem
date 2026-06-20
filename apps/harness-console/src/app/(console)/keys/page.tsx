"use client";

import * as React from "react";
import { KeyRound, Plug } from "lucide-react";
import { type HarnessKey } from "@/lib/harness";
import { PageHeader, Section } from "@/components/common/PageHeader";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { usePageToc } from "@/components/island/useScrollSpy";
import { InstallPanel } from "@/components/keys/InstallPanel";
import { KeyList } from "@/components/keys/KeyList";

/**
 * Keys + install: the cold-start hero. The install panel leads (a new user lands
 * here and connects a client in one paste), with the key list and create/revoke
 * controls below it. The tenant is baked into the key server side, so a client
 * only ever pastes the URL and the key.
 */
export default function KeysPage() {
  usePageToc();

  // The install panel binds to whichever key is "active". It starts on the most
  // recently created key once the list loads; creating a key promotes it here so
  // the snippet is filled the instant the key exists.
  const [activeKey, setActiveKey] = React.useState<HarnessKey | null>(null);

  return (
    <div className="mx-auto max-w-5xl px-6 py-8">
      <PageHeader
        eyebrow="inbound access"
        title="Keys"
        description="Connect a coding agent to the harness. Mint a scoped key, paste the install block into your client, and the harness is live. The tenant is bound to the key, so you only ever paste the URL and the key."
        actions={<Badge tone="neutral">URL + key, nothing else</Badge>}
      />

      <Section id="connect" title="Connect a client">
        <Card lift>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Plug size={16} className="text-ox" /> Install
            </CardTitle>
            <CardDescription>
              Pick your client and copy the exact setup block. Switching clients regenerates the snippet for the same
              key, so a connection is always one paste away.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <InstallPanel keyValue={activeKey} />
          </CardContent>
        </Card>
      </Section>

      <Section id="keys" title="Your keys">
        <KeyList
          onKeyCreated={(k) => {
            // Promote the freshly minted key so the install block above fills in.
            setActiveKey(k);
          }}
        />
      </Section>

      <Section id="scopes" title="What a key can do">
        <Card calm>
          <CardContent className="grid gap-3 pt-4 text-label text-muted-foreground sm:grid-cols-2">
            <ScopeNote
              icon={KeyRound}
              title="Scopes are least-privilege"
              body="A key carries only the scopes you pick: memory read/write, coordination, run, skills. CI keys can be read-only; an agent key gets write."
            />
            <ScopeNote
              icon={Plug}
              title="The tenant is server side"
              body="Your tenant is bound to the key when it's minted. Clients never set a tenant header; the harness resolves it from the bearer token."
            />
          </CardContent>
        </Card>
      </Section>
    </div>
  );
}

function ScopeNote({
  icon: Icon,
  title,
  body,
}: {
  icon: React.ComponentType<{ size?: number; className?: string }>;
  title: string;
  body: string;
}) {
  return (
    <div className="flex gap-3">
      <div className="mt-0.5 grid h-8 w-8 shrink-0 place-items-center rounded-full bg-surface-2 text-muted-foreground">
        <Icon size={15} />
      </div>
      <div>
        <p className="font-title text-body text-ink">{title}</p>
        <p className="mt-0.5">{body}</p>
      </div>
    </div>
  );
}
