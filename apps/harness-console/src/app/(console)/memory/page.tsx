"use client";

import * as React from "react";
import { Brain, ListFilter, Network, Orbit, Tag, Calendar, ChevronDown, Check, Database } from "lucide-react";
import {
  harness,
  type Atom,
  type AtomKind,
  type AtomLifecycle,
  type MemoryList,
  type SearchResult,
} from "@/lib/harness";
import { PageHeader, Section } from "@/components/common/PageHeader";
import { EmptyState } from "@/components/common/EmptyState";
import { SearchBar, type SearchMode } from "@/components/search/SearchBar";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { Popover, PopoverTrigger, PopoverContent } from "@/components/ui/popover";
import { Input } from "@/components/ui/input";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/misc";
import { useAsync } from "@/lib/hooks/useAsync";
import { usePageToc } from "@/components/island/useScrollSpy";
import { cn } from "@/lib/utils";

import { AtomTable } from "@/components/memory/AtomTable";
import { AtomEditor } from "@/components/memory/AtomEditor";
import { MemoryGraph } from "@/components/memory/MemoryGraph";
import { MemoryCluster } from "@/components/memory/MemoryCluster";
import { Dropzone } from "@/components/memory/Dropzone";

const KIND_OPTIONS: AtomKind[] = [
  "decision",
  "feedback",
  "solution",
  "postmortem",
  "preference",
  "note",
  "reflection",
  "handoff",
  "coordination",
  "source",
  "skill",
];

const VIEWS: { id: AtomLifecycle; label: string }[] = [
  { id: "active", label: "Active" },
  { id: "archived", label: "Archived" },
  { id: "trash", label: "Trash" },
];

type Mode = "list" | "graph" | "cluster";

export default function MemoryPage() {
  // Content surface: register the TOC so the Dynamic Island tracks sections.
  usePageToc();

  // ---- Filter + view state ------------------------------------------------
  const [view, setView] = React.useState<AtomLifecycle>("active");
  const [kinds, setKinds] = React.useState<AtomKind[]>([]);
  const [tag, setTag] = React.useState("");
  const [from, setFrom] = React.useState("");
  const [to, setTo] = React.useState("");
  const [query, setQuery] = React.useState("");
  const [searchMode, setSearchMode] = React.useState<SearchMode>("fulltext");
  const [mode, setMode] = React.useState<Mode>("list");

  // ---- Editor side-panel state (managed at page level) --------------------
  const [editorId, setEditorId] = React.useState<string | null>(null);
  const [editorAtom, setEditorAtom] = React.useState<Atom | null>(null);
  const [editorOpen, setEditorOpen] = React.useState(false);

  const tagList = React.useMemo(
    () => tag.split(",").map((t) => t.trim().replace(/^#/, "")).filter(Boolean),
    [tag],
  );

  // ---- Server-side filtered list ------------------------------------------
  const { data, loading, error, reload } = useAsync<MemoryList>(
    () =>
      harness.listMemory({
        view,
        kinds: kinds.length ? kinds : undefined,
        tags: tagList.length ? tagList : undefined,
        from: from || undefined,
        to: to || undefined,
      }),
    [view, kinds, tagList.join(","), from, to],
  );

  // ---- Search (full-text vs semantic), via harness.search -----------------
  const {
    data: searchHits,
    loading: searching,
  } = useAsync<SearchResult[] | null>(
    () => (query.trim() ? harness.search(query.trim(), searchMode) : Promise.resolve(null)),
    [query, searchMode],
  );

  // When a query is present, the search results gate the listed atoms (ordered
  // by the search ranking). Otherwise the filtered list stands on its own.
  const atoms = React.useMemo<Atom[]>(() => {
    const base = data?.atoms ?? [];
    if (!query.trim() || !searchHits) return base;
    const byId = new Map(base.map((a) => [a.id, a]));
    const order: Atom[] = [];
    for (const hit of searchHits) {
      const a = byId.get(hit.id);
      if (a) order.push(a);
    }
    return order;
  }, [data, searchHits, query]);

  const openAtom = (a: Atom) => {
    setEditorId(a.id);
    setEditorAtom(a);
    setEditorOpen(true);
    void harness.getAtom(a.id).then((hydrated) => {
      if (hydrated) {
        setEditorAtom((current) => (current?.id === hydrated.id ? hydrated : current));
      }
    });
  };

  // Open a wikilink target by title (resolve against the loaded set).
  const openLink = (title: string) => {
    const target = (data?.atoms ?? []).find((a) => a.title === title);
    if (target) openAtom(target);
  };

  const toggleKind = (k: AtomKind) =>
    setKinds((prev) => (prev.includes(k) ? prev.filter((x) => x !== k) : [...prev, k]));

  const clusters = data?.clusters ?? [];
  const edges = data?.edges ?? [];
  const showSearchEmpty = query.trim() && !searching && atoms.length === 0;

  return (
    <div>
      <PageHeader
        eyebrow="memory"
        title="Memory"
        description="The graph is the source of truth. Every view here is a projection; any node opens an editable markdown view."
      />

      {/* Search + filter bar ------------------------------------------------ */}
      <Section id="memory-browse" title="Browse">
        <div className="flex flex-col gap-3">
          <SearchBar
            value={query}
            onChange={setQuery}
            mode={searchMode}
            onModeChange={setSearchMode}
            placeholder="Search memory (text or semantic)..."
          />

          <div className="flex flex-wrap items-center gap-2">
            {/* Kind multi-select */}
            <Popover>
              <PopoverTrigger asChild>
                <button className="inline-flex h-9 items-center gap-2 rounded-md border border-line bg-bg px-3 font-mono text-label text-ink hover:bg-surface-2">
                  <ListFilter size={14} className="text-muted-foreground" />
                  {kinds.length ? `${kinds.length} kind${kinds.length > 1 ? "s" : ""}` : "kind"}
                  <ChevronDown size={13} className="text-muted-foreground" />
                </button>
              </PopoverTrigger>
              <PopoverContent className="max-h-72 overflow-y-auto">
                {KIND_OPTIONS.map((k) => {
                  const on = kinds.includes(k);
                  return (
                    <button
                      key={k}
                      onClick={() => toggleKind(k)}
                      className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left font-mono text-label text-ink hover:bg-surface-2"
                    >
                      <span
                        className={cn(
                          "grid h-4 w-4 place-items-center rounded border",
                          on ? "border-[var(--ox)] bg-[var(--ox-tint)] text-ox" : "border-line",
                        )}
                      >
                        {on && <Check size={11} />}
                      </span>
                      {k}
                    </button>
                  );
                })}
                {kinds.length > 0 && (
                  <button
                    onClick={() => setKinds([])}
                    className="mt-1 w-full rounded px-2 py-1 text-left font-mono text-[11px] text-ox hover:bg-[var(--ox-tint)]"
                  >
                    clear kinds
                  </button>
                )}
              </PopoverContent>
            </Popover>

            {/* Tag filter */}
            <div className="inline-flex h-9 items-center gap-2 rounded-md border border-line bg-bg px-3">
              <Tag size={14} className="text-muted-foreground" />
              <input
                value={tag}
                onChange={(e) => setTag(e.target.value)}
                placeholder="tags (comma-separated)"
                className="w-44 bg-transparent font-mono text-label text-ink outline-none placeholder:text-faint"
              />
            </div>

            {/* Date range */}
            <Popover>
              <PopoverTrigger asChild>
                <button className="inline-flex h-9 items-center gap-2 rounded-md border border-line bg-bg px-3 font-mono text-label text-ink hover:bg-surface-2">
                  <Calendar size={14} className="text-muted-foreground" />
                  {from || to ? `${from || "..."} - ${to || "..."}` : "date range"}
                  <ChevronDown size={13} className="text-muted-foreground" />
                </button>
              </PopoverTrigger>
              <PopoverContent className="w-64 p-3">
                <div className="flex flex-col gap-2">
                  <label className="font-mono text-[11px] text-muted-foreground">updated from</label>
                  <Input type="date" value={from} onChange={(e) => setFrom(e.target.value)} className="text-label" />
                  <label className="mt-1 font-mono text-[11px] text-muted-foreground">updated to</label>
                  <Input type="date" value={to} onChange={(e) => setTo(e.target.value)} className="text-label" />
                  {(from || to) && (
                    <button
                      onClick={() => {
                        setFrom("");
                        setTo("");
                      }}
                      className="mt-1 self-start font-mono text-[11px] text-ox hover:underline"
                    >
                      clear dates
                    </button>
                  )}
                </div>
              </PopoverContent>
            </Popover>

            {/* Active / Archived / Trash switch */}
            <div className="ml-auto inline-flex items-center gap-0.5 rounded-md border border-line p-0.5">
              {VIEWS.map((v) => (
                <button
                  key={v.id}
                  onClick={() => setView(v.id)}
                  className={cn(
                    "rounded px-2.5 py-1 font-mono text-[11px]",
                    view === v.id ? "bg-[var(--ox-tint)] text-ox" : "text-muted-foreground hover:text-ink",
                  )}
                >
                  {v.label}
                </button>
              ))}
            </div>
          </div>

          {/* Active filter chips */}
          {(kinds.length > 0 || tagList.length > 0) && (
            <div className="flex flex-wrap items-center gap-1.5">
              {kinds.map((k) => (
                <Badge key={k} tone="accent">
                  {k}
                </Badge>
              ))}
              {tagList.map((t) => (
                <Badge key={t} tone="neutral">
                  #{t}
                </Badge>
              ))}
            </div>
          )}
        </div>
      </Section>

      {/* Mode toggle + the active view ------------------------------------- */}
      <Section
        id="memory-view"
        title="View"
        actions={
          <Tabs value={mode} onValueChange={(v) => setMode(v as Mode)}>
            <TabsList>
              <TabsTrigger value="list">
                <ListFilter size={13} className="mr-1.5" /> List
              </TabsTrigger>
              <TabsTrigger value="graph">
                <Network size={13} className="mr-1.5" /> Graph
              </TabsTrigger>
              <TabsTrigger value="cluster">
                <Orbit size={13} className="mr-1.5" /> Cluster
              </TabsTrigger>
            </TabsList>
          </Tabs>
        }
      >
        {error ? (
          <EmptyState
            icon={Database}
            title="Memory failed to load"
            description={error}
          />
        ) : (
          <Tabs value={mode}>
            {/* List (default landing) */}
            <TabsContent value="list" className="focus-visible:outline-none">
              {loading ? (
                <div className="flex flex-col gap-2">
                  {Array.from({ length: 8 }).map((_, i) => (
                    <Skeleton key={i} className="h-10 w-full" />
                  ))}
                </div>
              ) : showSearchEmpty ? (
                <EmptyState
                  icon={Brain}
                  title={`No ${searchMode} matches for "${query.trim()}"`}
                  description="Try the other search mode, widen the filters, or switch the view."
                />
              ) : atoms.length === 0 ? (
                <EmptyState
                  icon={Brain}
                  title={`No ${view} atoms`}
                  description={
                    view === "active"
                      ? "Drop a file below or write a note to seed memory."
                      : `Nothing in ${view} matching the current filters.`
                  }
                />
              ) : (
                <Card calm className="overflow-hidden">
                  <AtomTable atoms={atoms} onOpen={openAtom} selectedId={editorOpen ? editorId : null} />
                  <div className="border-t border-line px-3 py-2 font-mono text-[11px] text-faint">
                    {atoms.length} atom{atoms.length === 1 ? "" : "s"}
                    {query.trim() && ` matching "${query.trim()}" (${searchMode})`}
                  </div>
                </Card>
              )}
            </TabsContent>

            {/* Graph (explore, opt-in) */}
            <TabsContent value="graph" className="focus-visible:outline-none">
              {loading ? (
                <Skeleton className="h-[60vh] w-full rounded-lg" />
              ) : atoms.length === 0 ? (
                <EmptyState icon={Network} title="No atoms to graph" description="Adjust the filters or view." />
              ) : (
                <div className="h-[60vh] w-full overflow-hidden rounded-lg border border-line bg-surface">
                  <MemoryGraph
                    atoms={atoms}
                    edges={edges}
                    clusters={clusters}
                    onNodeClick={openAtom}
                    className="h-full w-full"
                  />
                </div>
              )}
            </TabsContent>

            {/* Cluster (the recent.design projection) */}
            <TabsContent value="cluster" className="focus-visible:outline-none">
              <div className="h-[60vh] w-full">
                <MemoryCluster
                  atoms={atoms}
                  edges={edges}
                  clusters={clusters}
                  loading={loading}
                  onNodeClick={openAtom}
                  className="h-full w-full"
                />
              </div>
            </TabsContent>
          </Tabs>
        )}
      </Section>

      {/* Ingest ------------------------------------------------------------- */}
      <Section id="memory-ingest" title="Ingest">
        <Dropzone onIngested={reload} />
      </Section>

      {/* Shared editor side panel ------------------------------------------ */}
      <AtomEditor
        atom={editorAtom}
        open={editorOpen}
        onOpenChange={setEditorOpen}
        onMutated={reload}
        onOpenLink={openLink}
      />
    </div>
  );
}
