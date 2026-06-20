"use client";

import * as React from "react";
import { AlertTriangle, ScrollText, Sparkles } from "lucide-react";
import { harness, type Skill } from "@/lib/harness";
import { useAsync } from "@/lib/hooks/useAsync";
import { usePageToc } from "@/components/island/useScrollSpy";
import { PageHeader, Section } from "@/components/common/PageHeader";
import { EmptyState } from "@/components/common/EmptyState";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/misc";
import { SkillList } from "@/components/skills/SkillList";
import { SkillEditor } from "@/components/skills/SkillEditor";

/** Starter SKILL.md so a fresh pack opens valid and editable, not empty. */
const SKILL_TEMPLATE = `---
name: new-skill
description: Describe what this skill does and when to use it.
---

# New Skill

Use when ...

## Steps
1. ...
2. ...
`;

function draftSkill(): Skill {
  const id = `skill_draft_${Date.now().toString(36)}`;
  const now = new Date().toISOString();
  return {
    id,
    name: "new-skill",
    description: "Describe what this skill does and when to use it.",
    status: "draft",
    // Distinct from the content preview so a never-published draft reads dirty.
    contentHash: "sha256:unpublished",
    version: "v0",
    updated: now,
    files: [{ path: "SKILL.md", language: "markdown", content: SKILL_TEMPLATE }],
  };
}

export default function SkillsPage() {
  usePageToc();

  const { data, loading, error, reload } = useAsync(() => harness.listSkills(), []);

  // Local working copies layered over the loaded skills: edits live here until
  // publish, and a never-saved draft lives here too.
  const [drafts, setDrafts] = React.useState<Record<string, Skill>>({});
  const [selectedId, setSelectedId] = React.useState<string | null>(null);

  // The merged pack list: server skills with any local edit overlaid, plus
  // local-only drafts that have not been published yet.
  const merged = React.useMemo<Skill[]>(() => {
    const server = data ?? [];
    const serverIds = new Set(server.map((s) => s.id));
    const overlaid = server.map((s) => drafts[s.id] ?? s);
    const localOnly = Object.values(drafts).filter((s) => !serverIds.has(s.id));
    return [...localOnly, ...overlaid];
  }, [data, drafts]);

  // The published baselines, used to compute the dirty set.
  const baselines = React.useMemo<Record<string, Skill>>(() => {
    const map: Record<string, Skill> = {};
    for (const s of data ?? []) map[s.id] = s;
    return map;
  }, [data]);

  const dirtyIds = React.useMemo(() => {
    const set = new Set<string>();
    for (const s of merged) {
      const base = baselines[s.id];
      if (!base) {
        set.add(s.id); // local-only draft, never published
        continue;
      }
      const edited = drafts[s.id];
      if (edited && JSON.stringify(serialize(edited)) !== JSON.stringify(serialize(base))) {
        set.add(s.id);
      }
    }
    return set;
  }, [merged, baselines, drafts]);

  // Default the selection to the first pack once data lands.
  React.useEffect(() => {
    if (selectedId) return;
    if (merged.length) setSelectedId(merged[0].id);
  }, [merged, selectedId]);

  const selected = merged.find((s) => s.id === selectedId) ?? null;

  const createSkill = () => {
    const skill = draftSkill();
    setDrafts((d) => ({ ...d, [skill.id]: skill }));
    setSelectedId(skill.id);
  };

  const editSkill = (next: Skill) => {
    setDrafts((d) => ({ ...d, [next.id]: next }));
  };

  const onPublished = (published: Skill) => {
    // Clear the local edit (server is now authoritative) and refetch the list.
    setDrafts((d) => {
      const { [published.id]: _drop, ...rest } = d;
      void _drop;
      return rest;
    });
    setSelectedId(published.id);
    reload();
  };

  return (
    <div className="mx-auto max-w-[1280px]">
      <PageHeader
        eyebrow="authoring"
        title="Skills"
        description="Author, version, and publish content-addressed skill packs. Each pack is a SKILL.md plus its files, promoted up a lifecycle from draft to canonical."
        actions={
          <Button variant="primary" size="sm" onClick={createSkill}>
            <Sparkles size={14} /> new skill
          </Button>
        }
      />

      <Section id="packs" title="Skill packs">
        {loading && !data ? (
          <LoadingState />
        ) : error ? (
          <ErrorState message={error} onRetry={reload} />
        ) : merged.length === 0 ? (
          <EmptyState
            icon={ScrollText}
            title="Create your first skill"
            description="A skill is a SKILL.md with name + description frontmatter, plus any supporting files. Start from a template and publish to get a content hash."
            action={
              <Button variant="primary" size="sm" onClick={createSkill}>
                <Sparkles size={14} /> new SKILL.md
              </Button>
            }
          />
        ) : (
          <div className="grid gap-6 lg:grid-cols-[320px_1fr]">
            {/* left: pack list */}
            <div className="lg:max-h-[calc(100vh-220px)]">
              <SkillList
                skills={merged}
                selectedId={selectedId}
                dirtyIds={dirtyIds}
                onSelect={setSelectedId}
                onCreate={createSkill}
              />
            </div>

            {/* right: editor */}
            <div className="material min-h-[560px] p-4 lg:max-h-[calc(100vh-220px)] lg:overflow-y-auto">
              {selected ? (
                <SkillEditor
                  key={selected.id}
                  skill={selected}
                  onChange={editSkill}
                  onPublished={onPublished}
                />
              ) : (
                <div className="grid h-full place-items-center font-mono text-label text-muted-foreground">
                  Select a pack to edit.
                </div>
              )}
            </div>
          </div>
        )}
      </Section>
    </div>
  );
}

/** Compare only the content-bearing fields when deciding dirty. */
function serialize(s: Skill) {
  return {
    name: s.name,
    description: s.description,
    status: s.status,
    files: s.files.map((f) => ({ path: f.path, content: f.content })),
  };
}

function LoadingState() {
  return (
    <div className="grid gap-6 lg:grid-cols-[320px_1fr]">
      <div className="space-y-2">
        {Array.from({ length: 4 }).map((_, i) => (
          <Skeleton key={i} className="h-24 w-full rounded-lg" />
        ))}
      </div>
      <Skeleton className="h-[560px] w-full rounded-lg" />
    </div>
  );
}

function ErrorState({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="material flex flex-col items-center justify-center gap-3 px-6 py-12 text-center">
      <div className="grid h-11 w-11 place-items-center rounded-full bg-[var(--ox-tint)] text-ox">
        <AlertTriangle size={20} />
      </div>
      <div>
        <p className="font-title text-subhead text-ink">Could not load skills</p>
        <p className="mt-1 max-w-md font-mono text-label text-muted-foreground">{message}</p>
      </div>
      <Button variant="outline" size="sm" onClick={onRetry}>
        Retry
      </Button>
    </div>
  );
}
