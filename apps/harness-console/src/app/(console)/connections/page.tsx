"use client";

import { PageHeader, Section } from "@/components/common/PageHeader";
import { usePageToc } from "@/components/island/useScrollSpy";
import { GithubConnection } from "@/components/connections/GithubConnection";
import { McpHub } from "@/components/connections/McpHub";

/**
 * Connections + MCP Hub (Settings).
 *
 * Two parts: the GitHub connection (authorize repos -> ingest into the code
 * graph) and the MCP Hub (the harness as the single MCP endpoint coding agents
 * connect to: namespaced capabilities + brokered servers + one-connection
 * install snippets).
 */
export default function ConnectionsPage() {
  usePageToc();

  return (
    <div>
      <PageHeader
        eyebrow="settings"
        title="Connections"
        description="Wire the harness to your sources and your agents. Connect GitHub so the code graph stays fresh, and give every coding agent one MCP endpoint."
      />

      <Section
        id="github"
        title="GitHub"
        depth={2}
      >
        <GithubConnection />
      </Section>

      <Section
        id="mcp-hub"
        title="MCP Hub"
        depth={2}
      >
        <McpHub />
      </Section>
    </div>
  );
}
