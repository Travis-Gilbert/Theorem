"use client";

import * as React from "react";
import { Search, Type, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";

export type SearchMode = "fulltext" | "semantic";

/**
 * Shared search component with a toggle between full-text and semantic. Memory
 * uses it; the omnibar reuses the same binding. Full-text binds
 * rustyred_thg_fulltext_search; semantic binds rustyred_thg_vector_search /
 * hippo_retrieve (wired through the harness client's search()).
 */
export function SearchBar({
  value,
  onChange,
  mode,
  onModeChange,
  placeholder = "Search memory...",
  className,
}: {
  value: string;
  onChange: (v: string) => void;
  mode: SearchMode;
  onModeChange: (m: SearchMode) => void;
  placeholder?: string;
  className?: string;
}) {
  return (
    <div className={cn("flex items-center gap-2", className)}>
      <div className="flex h-9 flex-1 items-center gap-2 rounded-md border border-line bg-bg px-3">
        <Search size={15} className="text-muted-foreground" />
        <input
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          className="w-full bg-transparent text-body text-ink outline-none placeholder:text-faint"
        />
      </div>
      <div className="flex items-center gap-0.5 rounded-md border border-line p-0.5" role="tablist" aria-label="Search mode">
        <button
          role="tab"
          aria-selected={mode === "fulltext"}
          onClick={() => onModeChange("fulltext")}
          title="Full-text"
          className={cn(
            "flex items-center gap-1 rounded px-2 py-1 font-mono text-[11px]",
            mode === "fulltext" ? "bg-[var(--ox-tint)] text-ox" : "text-muted-foreground hover:text-ink",
          )}
        >
          <Type size={12} /> text
        </button>
        <button
          role="tab"
          aria-selected={mode === "semantic"}
          onClick={() => onModeChange("semantic")}
          title="Semantic"
          className={cn(
            "flex items-center gap-1 rounded px-2 py-1 font-mono text-[11px]",
            mode === "semantic" ? "bg-[var(--ox-tint)] text-ox" : "text-muted-foreground hover:text-ink",
          )}
        >
          <Sparkles size={12} /> semantic
        </button>
      </div>
    </div>
  );
}
