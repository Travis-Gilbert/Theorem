"use client";

import * as React from "react";
import { ArrowDown, ArrowUp } from "lucide-react";
import type { Atom } from "@/lib/harness";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from "@/components/ui/table";
import { cn, relativeTime } from "@/lib/utils";

type SortKey = "title" | "kind" | "score" | "updated";
type SortDir = "asc" | "desc";

// Kind chips lean neutral; the rarer, load-bearing kinds get accent so they
// stand out against the coordination exhaust that dominates the corpus.
const ACCENT_KINDS = new Set<Atom["kind"]>(["decision", "solution", "postmortem", "handoff"]);
function kindTone(kind: Atom["kind"]): BadgeProps["tone"] {
  if (kind === "skill") return "ink";
  if (ACCENT_KINDS.has(kind)) return "accent";
  return "neutral";
}

/** The score column shows fitness for learned/skill atoms, salience otherwise. */
function scoreOf(a: Atom): { value: number; label: string } {
  return typeof a.fitness === "number"
    ? { value: a.fitness, label: "fitness" }
    : { value: a.salience, label: "salience" };
}

/** A sortable column header. Declared at module scope so it is not recreated on
 *  each render (and so its state, if any, is stable). */
function SortHead({
  k,
  children,
  className,
  sortKey,
  sortDir,
  onSort,
}: {
  k: SortKey;
  children: React.ReactNode;
  className?: string;
  sortKey: SortKey;
  sortDir: SortDir;
  onSort: (k: SortKey) => void;
}) {
  const active = sortKey === k;
  return (
    <TableHead
      className={className}
      aria-sort={active ? (sortDir === "asc" ? "ascending" : "descending") : "none"}
    >
      <button onClick={() => onSort(k)} className="inline-flex items-center gap-1 hover:text-ink">
        {children}
        {active && (sortDir === "asc" ? <ArrowUp size={11} /> : <ArrowDown size={11} />)}
      </button>
    </TableHead>
  );
}

/**
 * List mode: the scannable table that is the Memory landing. A graph of 236
 * atoms (mostly coordination exhaust) is unreadable as a default, so the table
 * leads. Sortable columns, a salience/fitness bar, and a whole-row click that
 * opens the shared editor.
 */
export function AtomTable({
  atoms,
  onOpen,
  selectedId,
}: {
  atoms: Atom[];
  onOpen: (atom: Atom) => void;
  selectedId?: string | null;
}) {
  const [sortKey, setSortKey] = React.useState<SortKey>("updated");
  const [sortDir, setSortDir] = React.useState<SortDir>("desc");

  const sorted = React.useMemo(() => {
    const dir = sortDir === "asc" ? 1 : -1;
    return [...atoms].sort((a, b) => {
      switch (sortKey) {
        case "title":
          return a.title.localeCompare(b.title) * dir;
        case "kind":
          return a.kind.localeCompare(b.kind) * dir;
        case "score":
          return (scoreOf(a).value - scoreOf(b).value) * dir;
        case "updated":
        default:
          return (new Date(a.updated).getTime() - new Date(b.updated).getTime()) * dir;
      }
    });
  }, [atoms, sortKey, sortDir]);

  const toggleSort = (key: SortKey) => {
    if (key === sortKey) setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    else {
      setSortKey(key);
      setSortDir(key === "title" || key === "kind" ? "asc" : "desc");
    }
  };

  const headProps = { sortKey, sortDir, onSort: toggleSort };

  return (
    <Table>
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          <SortHead k="title" {...headProps}>title</SortHead>
          <SortHead k="kind" {...headProps}>kind</SortHead>
          <TableHead className="hidden lg:table-cell">summary</TableHead>
          <TableHead className="hidden md:table-cell">tags</TableHead>
          <SortHead k="score" className="w-32" {...headProps}>score</SortHead>
          <SortHead k="updated" className="w-24 text-right" {...headProps}>updated</SortHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {sorted.map((a) => {
          const score = scoreOf(a);
          return (
            <TableRow
              key={a.id}
              data-selected={a.id === selectedId}
              onClick={() => onOpen(a)}
              tabIndex={0}
              role="button"
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onOpen(a);
                }
              }}
              className="cursor-pointer"
            >
              <TableCell className="max-w-[18rem]">
                <span className="block truncate font-title text-body text-ink" title={a.title}>
                  {a.title}
                </span>
              </TableCell>
              <TableCell>
                <Badge tone={kindTone(a.kind)}>{a.kind}</Badge>
              </TableCell>
              <TableCell className="hidden max-w-[24rem] lg:table-cell">
                <span className="block truncate text-label text-muted-foreground" title={a.summary}>
                  {a.summary}
                </span>
              </TableCell>
              <TableCell className="hidden md:table-cell">
                <div className="flex flex-wrap gap-1">
                  {a.tags.slice(0, 3).map((t, i) => (
                    <span key={`${t}-${i}`} className="font-mono text-[11px] text-faint">
                      #{t}
                    </span>
                  ))}
                  {a.tags.length > 3 && (
                    <span className="font-mono text-[11px] text-faint">+{a.tags.length - 3}</span>
                  )}
                </div>
              </TableCell>
              <TableCell>
                <div className="flex items-center gap-2" title={`${score.label} ${score.value.toFixed(2)}`}>
                  <span className="h-1.5 w-16 overflow-hidden rounded-full bg-surface-2">
                    <span
                      className={cn("block h-full rounded-full", typeof a.fitness === "number" ? "bg-live" : "bg-ox")}
                      style={{ width: `${Math.round(score.value * 100)}%` }}
                    />
                  </span>
                  <span className="font-mono text-[11px] text-muted-foreground">{score.value.toFixed(2)}</span>
                </div>
              </TableCell>
              <TableCell className="text-right">
                <span className="font-mono text-[11px] text-faint">{relativeTime(a.updated)}</span>
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}
