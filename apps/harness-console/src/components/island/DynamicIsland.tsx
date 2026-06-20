"use client";

import * as React from "react";
import { useRouter, usePathname } from "next/navigation";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import {
  Search,
  Globe,
  Command as CommandIcon,
  Paperclip,
  ArrowUp,
  List,
  Network,
  Hash,
  AtSign,
  ChevronRight,
  Sparkles,
} from "lucide-react";
import { useConsole } from "./console-context";
import { harness, type SearchResult } from "@/lib/harness";
import { cn } from "@/lib/utils";

/**
 * The Dynamic Island TOC + omnibar. One element, bottom-center, permanent in the
 * bottom third. It is the omnibar, the command spine, the table of contents, the
 * ambient context bar, and the RustyWeb search box, unified by the Dynamic
 * Island metaphor (the iOS control-surface metaphor too). Treatment follows the
 * reuno-ui ai-input: a rounded pill, an attach affordance, a search toggle, a
 * send, retokenized to white/grey/black/oxblood with oxblood for active search
 * and send.
 *
 * States: ambient (collapsed) -> expanded (TOC / cluster list) -> search ->
 * command palette. Cmd/Ctrl K opens the palette from any surface; Escape resets
 * to the ambient pill (handled in console-context).
 */

const NAV = [
  { id: "canvas", title: "Canvas", href: "/canvas" },
  { id: "inbox", title: "Inbox", href: "/inbox" },
  { id: "agent", title: "Agent", href: "/agent" },
  { id: "memory", title: "Memory", href: "/memory" },
  { id: "skills", title: "Skills", href: "/skills" },
  { id: "rooms", title: "Rooms", href: "/rooms" },
  { id: "runs", title: "Runs", href: "/runs" },
  { id: "keys", title: "API Keys", href: "/keys" },
  { id: "providers", title: "Providers", href: "/providers" },
  { id: "usage", title: "Usage", href: "/usage" },
  { id: "connections", title: "Connections", href: "/connections" },
  { id: "settings", title: "Settings", href: "/settings" },
];

const VERBS = [
  { id: "encode", title: "encode", hint: "record a memory signal" },
  { id: "recall", title: "recall", hint: "search the memory graph" },
  { id: "spawn", title: "spawn", hint: "start a composed agent run" },
  { id: "replay", title: "replay", hint: "walk a run timeline" },
  { id: "search", title: "search", hint: "graph + RustyWeb search" },
];

const TAGS = ["#decision", "#feedback", "#postmortem", "#rustyred", "#graphql", "#deploy"];

function ProgressRing({ progress }: { progress: number }) {
  const r = 8;
  const c = 2 * Math.PI * r;
  return (
    <svg width="20" height="20" viewBox="0 0 20 20" className="-rotate-90">
      <circle cx="10" cy="10" r={r} fill="none" stroke="var(--line)" strokeWidth="2" />
      <circle
        cx="10"
        cy="10"
        r={r}
        fill="none"
        stroke="var(--ox)"
        strokeWidth="2"
        strokeLinecap="round"
        strokeDasharray={c}
        strokeDashoffset={c * (1 - progress)}
      />
    </svg>
  );
}

type Mode = "ambient" | "expanded" | "search" | "command";

export function DynamicIsland() {
  const router = useRouter();
  const pathname = usePathname();
  const reduced = useReducedMotion();
  const {
    paletteOpen,
    setPaletteOpen,
    searchOn,
    setSearchOn,
    activeSection,
    progress,
    toc,
    surfaceMode,
    clusters,
    hoverNode,
  } = useConsole();

  const [query, setQuery] = React.useState("");
  const [results, setResults] = React.useState<SearchResult[]>([]);
  const [resultView, setResultView] = React.useState<"list" | "graph">("list");
  const inputRef = React.useRef<HTMLInputElement>(null);

  // Onboarding routes hide the island; it belongs to the authed console.
  const hidden = pathname?.startsWith("/claim");

  const mode: Mode = paletteOpen ? "command" : searchOn ? "search" : "ambient";

  React.useEffect(() => {
    if (mode === "command" || mode === "search") {
      inputRef.current?.focus();
    }
  }, [mode]);

  // Resolve typed input three ways for the palette, plus RustyWeb search.
  const runQuery = React.useCallback(
    async (q: string) => {
      setQuery(q);
      if (!q.trim()) {
        setResults([]);
        return;
      }
      if (mode === "search") {
        const hits = await harness.search(q, "semantic");
        setResults(hits);
        return;
      }
      if (q.startsWith(">")) {
        const term = q.slice(1).trim().toLowerCase();
        setResults(
          VERBS.filter((v) => v.title.includes(term)).map((v) => ({
            id: v.id,
            kind: "action",
            title: `> ${v.title}`,
            subtitle: v.hint,
          })),
        );
      } else if (q.startsWith("@")) {
        const term = q.slice(1).trim().toLowerCase();
        setResults(
          NAV.filter((n) => n.title.toLowerCase().includes(term)).map((n) => ({
            id: n.id,
            kind: "action",
            title: `@ ${n.title}`,
            href: n.href,
          })),
        );
      } else if (q.startsWith("#")) {
        const term = q.slice(1).trim().toLowerCase();
        setResults(
          TAGS.filter((t) => t.includes(term)).map((t) => ({ id: t, kind: "action", title: t })),
        );
      } else {
        const hits = await harness.search(q, "fulltext");
        setResults([
          ...hits,
          { id: "ask", kind: "action", title: `Ask the Theorem agent: "${q}"`, href: `/agent?prompt=${encodeURIComponent(q)}` },
        ]);
      }
    },
    [mode],
  );

  function choose(r: SearchResult) {
    if (r.href) router.push(r.href);
    setPaletteOpen(false);
    setSearchOn(false);
    setQuery("");
    setResults([]);
  }

  if (hidden) return null;

  const ambientLabel = hoverNode
    ? hoverNode.title
    : activeSection
      ? toc.find((t) => t.id === activeSection)?.title ?? "Console"
      : NAV.find((n) => pathname?.startsWith(n.href))?.title ?? "Theorems Harness";

  const expandList =
    surfaceMode === "memory"
      ? clusters.map((c) => ({ id: c.id, title: c.label }))
      : toc.map((t) => ({ id: t.id, title: t.title }));

  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-6 z-[60] flex justify-center px-4">
      <AnimatePresence>
        {mode !== "ambient" && (
          <motion.div
            key="backdrop"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="backdrop-overlay pointer-events-auto fixed inset-0 -z-10"
            onClick={() => {
              setPaletteOpen(false);
              setSearchOn(false);
            }}
          />
        )}
      </AnimatePresence>

      <motion.div
        layout={!reduced}
        transition={reduced ? { duration: 0 } : { type: "spring", stiffness: 380, damping: 32 }}
        className={cn(
          "pointer-events-auto w-full max-w-2xl overflow-hidden rounded-2xl border border-line bg-bg",
          mode === "ambient" ? "elev-2" : "elev-3",
        )}
      >
        {/* Expanded panel content (TOC / cluster list / command results) */}
        <AnimatePresence initial={false}>
          {mode !== "ambient" && (
            <motion.div
              initial={reduced ? false : { height: 0, opacity: 0 }}
              animate={{ height: "auto", opacity: 1 }}
              exit={reduced ? undefined : { height: 0, opacity: 0 }}
              className="max-h-[46vh] overflow-y-auto border-b border-line"
            >
              {mode === "search" ? (
                <SearchPanel results={results} view={resultView} setView={setResultView} onChoose={choose} />
              ) : (
                <CommandPanel query={query} results={results} expandList={expandList} surfaceMode={surfaceMode} onChoose={choose} router={router} setPaletteOpen={setPaletteOpen} />
              )}
            </motion.div>
          )}
        </AnimatePresence>

        {/* The pill row, always present */}
        <div className="flex items-center gap-2 px-3 py-2">
          {mode === "ambient" ? (
            <button
              className="flex flex-1 items-center gap-3 rounded-xl px-2 py-1 text-left hover:bg-surface-2"
              onClick={() => setPaletteOpen(true)}
              aria-label="Open command palette"
            >
              <ProgressRing progress={hoverNode ? 1 : progress} />
              <span className="flex-1 truncate font-mono text-label text-ink">{ambientLabel}</span>
              {hoverNode?.meta ? (
                <span className="truncate font-mono text-[11px] text-muted-foreground">{hoverNode.meta}</span>
              ) : (
                <span className="rail-group-label">{surfaceMode === "memory" ? "clusters" : "sections"}</span>
              )}
            </button>
          ) : (
            <>
              <button
                className={cn("rounded-lg p-1.5 transition-colors", searchOn ? "text-ox" : "text-muted-foreground hover:text-ink")}
                onClick={() => {
                  setSearchOn(!searchOn);
                  setPaletteOpen(false);
                }}
                aria-pressed={searchOn}
                aria-label="Toggle RustyWeb search"
                title={searchOn ? "RustyWeb search: on" : "RustyWeb search: off"}
              >
                <Globe size={16} />
              </button>
              <input
                ref={inputRef}
                value={query}
                onChange={(e) => runQuery(e.target.value)}
                placeholder={mode === "search" ? "Search RustyWeb..." : "Type a command, > verb, @ nav, # tag, or ask..."}
                className="flex-1 bg-transparent font-mono text-body text-ink outline-none placeholder:text-faint"
              />
              <button className="rounded-lg p-1.5 text-muted-foreground hover:text-ink" aria-label="Attach context">
                <Paperclip size={15} />
              </button>
              <button
                className="rounded-lg bg-ox p-1.5 text-white hover:bg-[#73241f]"
                aria-label="Submit"
                onClick={() => results[0] && choose(results[0])}
              >
                <ArrowUp size={15} />
              </button>
            </>
          )}
          {mode === "ambient" && (
            <div className="flex items-center gap-1">
              <button
                className="rounded-lg p-1.5 text-muted-foreground hover:bg-surface-2 hover:text-ox"
                onClick={() => {
                  setSearchOn(true);
                  setPaletteOpen(false);
                }}
                aria-label="Search"
              >
                <Search size={15} />
              </button>
              <button
                className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-muted-foreground hover:bg-surface-2 hover:text-ink"
                onClick={() => setPaletteOpen(true)}
                aria-label="Command palette"
              >
                <CommandIcon size={13} />
                <kbd className="font-mono text-[10px]">K</kbd>
              </button>
            </div>
          )}
        </div>
      </motion.div>
    </div>
  );
}

function SearchPanel({
  results,
  view,
  setView,
  onChoose,
}: {
  results: SearchResult[];
  view: "list" | "graph";
  setView: (v: "list" | "graph") => void;
  onChoose: (r: SearchResult) => void;
}) {
  return (
    <div className="p-2">
      <div className="mb-2 flex items-center justify-between px-1">
        <span className="rail-group-label">RustyWeb results</span>
        <div className="flex items-center gap-1 rounded-md border border-line p-0.5">
          <button
            className={cn("rounded px-2 py-0.5", view === "list" ? "bg-surface-2 text-ink" : "text-muted-foreground")}
            onClick={() => setView("list")}
          >
            <List size={13} />
          </button>
          <button
            className={cn("rounded px-2 py-0.5", view === "graph" ? "bg-surface-2 text-ink" : "text-muted-foreground")}
            onClick={() => setView("graph")}
          >
            <Network size={13} />
          </button>
        </div>
      </div>
      {view === "list" ? (
        <ul className="flex flex-col">
          {results.map((r) => (
            <li key={r.id}>
              <button
                onClick={() => onChoose(r)}
                className="flex w-full items-center gap-2 rounded px-2 py-2 text-left hover:bg-surface-2"
              >
                <span className="flex-1 truncate text-body text-ink">{r.title}</span>
                {r.score != null && <span className="font-mono text-[11px] text-muted-foreground">{r.score.toFixed(2)}</span>}
              </button>
            </li>
          ))}
          {!results.length && <li className="px-2 py-6 text-center text-label text-muted-foreground">Submit a query to search RustyWeb.</li>}
        </ul>
      ) : (
        <div className="grid h-44 place-items-center rounded-md border border-dashed border-line text-label text-muted-foreground">
          <div className="text-center">
            <Network size={20} className="mx-auto mb-1 text-faint" />
            results graph (cosmos.gl cloud, recent.design style)
          </div>
        </div>
      )}
    </div>
  );
}

function CommandPanel({
  query,
  results,
  expandList,
  surfaceMode,
  onChoose,
  router,
  setPaletteOpen,
}: {
  query: string;
  results: SearchResult[];
  expandList: { id: string; title: string }[];
  surfaceMode: string;
  onChoose: (r: SearchResult) => void;
  router: ReturnType<typeof useRouter>;
  setPaletteOpen: (v: boolean) => void;
}) {
  // No query yet: show the TOC (content) or cluster list (memory) as the
  // expanded state, plus the prefix legend.
  if (!query.trim()) {
    return (
      <div className="p-2">
        <div className="mb-1 flex items-center gap-3 px-2 py-1 text-[11px] text-muted-foreground">
          <span className="flex items-center gap-1"><ChevronRight size={11} />verb</span>
          <span className="flex items-center gap-1"><AtSign size={11} />nav</span>
          <span className="flex items-center gap-1"><Hash size={11} />tag</span>
          <span className="flex items-center gap-1"><Sparkles size={11} />ask</span>
        </div>
        <div className="rail-group-label px-2 py-1">{surfaceMode === "memory" ? "Clusters" : "On this page"}</div>
        <ul>
          {expandList.map((e) => (
            <li key={e.id}>
              <button
                onClick={() => {
                  document.getElementById(e.id)?.scrollIntoView({ behavior: "smooth" });
                  setPaletteOpen(false);
                }}
                className="w-full truncate rounded px-2 py-1.5 text-left text-body text-ink hover:bg-surface-2"
              >
                {e.title}
              </button>
            </li>
          ))}
          {!expandList.length && (
            <li className="px-2 py-3 text-label text-muted-foreground">Type to search, or use a prefix above.</li>
          )}
        </ul>
      </div>
    );
  }
  return (
    <ul className="p-2">
      {results.map((r) => (
        <li key={r.id}>
          <button onClick={() => onChoose(r)} className="flex w-full flex-col gap-0.5 rounded px-2 py-2 text-left hover:bg-surface-2">
            <span className="truncate text-body text-ink">{r.title}</span>
            {r.subtitle && <span className="truncate text-label text-muted-foreground">{r.subtitle}</span>}
          </button>
        </li>
      ))}
      {!results.length && <li className="px-2 py-6 text-center text-label text-muted-foreground">No matches.</li>}
    </ul>
  );
}
