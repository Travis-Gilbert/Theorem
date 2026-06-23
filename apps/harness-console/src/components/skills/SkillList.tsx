"use client";

import * as React from "react";
import { CircleDot, Hash, Plus } from "lucide-react";
import { cn, relativeTime } from "@/lib/utils";
import type { Skill } from "@/lib/harness";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { statusTone } from "./SkillEditor";

export function SkillList({
  skills,
  selectedId,
  dirtyIds,
  onSelect,
  onCreate,
}: {
  skills: Skill[];
  selectedId: string | null;
  /** ids whose local edits differ from the published content hash. */
  dirtyIds?: Set<string>;
  onSelect: (id: string) => void;
  onCreate: () => void;
}) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center justify-between px-1 pb-3">
        <span className="rail-group-label">{skills.length} packs</span>
        <Button variant="outline" size="sm" onClick={onCreate}>
          <Plus size={14} /> new
        </Button>
      </div>

      <ul className="min-h-0 flex-1 space-y-2 overflow-y-auto pr-1">
        {skills.map((skill) => {
          const active = skill.id === selectedId;
          const dirty = dirtyIds?.has(skill.id) ?? false;
          return (
            <li key={skill.id}>
              <button
                type="button"
                onClick={() => onSelect(skill.id)}
                aria-current={active}
                className={cn(
                  "material w-full p-3 text-left transition-colors",
                  active
                    ? "ring-1 ring-[var(--ox)]"
                    : "material-lift hover:border-[var(--line)]",
                )}
              >
                <div className="flex items-start justify-between gap-2">
                  <span className="truncate font-mono text-label font-medium text-ink">{skill.name}</span>
                  <Badge tone={statusTone(skill.status)} className="shrink-0">
                    {skill.status}
                  </Badge>
                </div>
                <p className="mt-1 line-clamp-2 text-label text-muted-foreground">{skill.description}</p>
                <div className="mt-2 flex items-center gap-x-3 gap-y-1 font-mono text-[11px] text-faint">
                  <span className="inline-flex items-center gap-1 truncate">
                    <Hash size={10} className="shrink-0" />
                    {skill.contentHash}
                  </span>
                  <span className="shrink-0">{skill.version}</span>
                  <span className="ml-auto shrink-0">{relativeTime(skill.updated)}</span>
                  {dirty && (
                    <span className="inline-flex shrink-0 items-center gap-0.5 text-warn" title="Unpublished edits">
                      <CircleDot size={10} />
                    </span>
                  )}
                </div>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
