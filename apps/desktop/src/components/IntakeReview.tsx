import { useEffect, useMemo, useState, type CSSProperties } from "react";
import {
  CheckIcon,
  QueueIcon,
  RouteIcon,
  SourceIcon,
  TaskIcon,
} from "./icons";

type SourceKind = "gmail" | "drive" | "notion" | "linear" | "outlook";
type FilterKind = SourceKind | "all";
type IntakeState = "needs_you" | "review" | "routed";

interface IntakeTask {
  title: string;
  state: "open" | "blocked" | "done";
  due: string;
}

interface Writeback {
  actor: string;
  status: string;
  at: string;
}

interface IntakeItem {
  id: string;
  source: SourceKind;
  sourceLabel: string;
  sourceContainer: string;
  externalId: string;
  title: string;
  from: string;
  fetchedAt: string;
  preview: string;
  body: string;
  collection: string;
  confidence: number;
  contentScore: number;
  sourcePrior: number;
  state: IntakeState;
  reason: string;
  priority: "high" | "normal" | "low";
  due: string;
  graph: string[];
  tasks: IntakeTask[];
  writebacks: Writeback[];
}

const INITIAL_ITEMS: IntakeItem[] = [
  {
    id: "gmail-brand-review",
    source: "gmail",
    sourceLabel: "Gmail",
    sourceContainer: "Inbox / Clients",
    externalId: "gm-7b1d-184",
    title: "Brand review notes from Mara Ellingsen",
    from: "Mara Ellingsen",
    fetchedAt: "9:42",
    preview:
      "Mara sent the revised partner deck, two unresolved pricing notes, and a request for a Friday answer.",
    body:
      "The revised deck is ready for review. The two open items are the package naming on slide 12 and the partner pricing language. Mara needs a clean answer before the Friday planning block.",
    collection: "Projects / Partner Launch",
    confidence: 0.68,
    contentScore: 0.61,
    sourcePrior: 0.07,
    state: "needs_you",
    reason: "Ambiguous between Partner Launch and Brand System",
    priority: "high",
    due: "Fri 11:30",
    graph: ["Partner Launch", "Brand System", "Mara Ellingsen"],
    tasks: [
      { title: "Resolve slide 12 naming", state: "open", due: "Fri" },
      { title: "Answer pricing language", state: "open", due: "Fri" },
    ],
    writebacks: [
      { actor: "Routing agent", status: "Filed for review", at: "9:43" },
    ],
  },
  {
    id: "linear-onboarding",
    source: "linear",
    sourceLabel: "Linear",
    sourceContainer: "CommonPlace / Current",
    externalId: "CP-482",
    title: "Onboarding checklist needs account-source mapping",
    from: "Noemi Ibarra",
    fetchedAt: "10:08",
    preview:
      "Linear issue maps directly to a task, with dependencies on Gmail and Drive source scopes.",
    body:
      "The checklist should distinguish account connection, first sync, and first review decision. It depends on the Gmail and Drive source scopes landing in the same graph vocabulary.",
    collection: "Product / CommonPlace Desktop",
    confidence: 0.81,
    contentScore: 0.77,
    sourcePrior: 0.04,
    state: "routed",
    reason: "Hard source rule matched Linear current work",
    priority: "normal",
    due: "Mon 14:00",
    graph: ["CommonPlace Desktop", "Source Accounts", "CP-482"],
    tasks: [
      { title: "Draft account-source mapping", state: "open", due: "Mon" },
    ],
    writebacks: [
      { actor: "Routing agent", status: "Auto-filed", at: "10:09" },
      { actor: "Task agent", status: "Created task edge", at: "10:09" },
    ],
  },
  {
    id: "drive-model-notes",
    source: "drive",
    sourceLabel: "Drive",
    sourceContainer: "Shared / Research",
    externalId: "drv-42c9",
    title: "Model-evaluation notes from the June field session",
    from: "Shared Drive",
    fetchedAt: "11:26",
    preview:
      "One field document looks related to evaluation protocol, but the source folder carries a research prior.",
    body:
      "The document compares three evaluation protocols and flags one method as too brittle for source-origin routing. It references prior work on graph-first intake and live review loops.",
    collection: "Research / Evaluation Protocols",
    confidence: 0.73,
    contentScore: 0.69,
    sourcePrior: 0.04,
    state: "review",
    reason: "Confidence sits inside review band",
    priority: "low",
    due: "No due date",
    graph: ["Evaluation Protocols", "Graph Intake", "Field Session"],
    tasks: [],
    writebacks: [
      { actor: "Routing agent", status: "Queued reviewer check", at: "11:28" },
    ],
  },
  {
    id: "notion-source-contract",
    source: "notion",
    sourceLabel: "Notion",
    sourceContainer: "Source Catalog",
    externalId: "ntn-886f",
    title: "Source contract table updated with Outlook scope",
    from: "Catalog sync",
    fetchedAt: "12:14",
    preview:
      "The source catalog changed a field-map row and introduced a container-specific Outlook rule.",
    body:
      "The catalog now maps Outlook conversation folders into source_container values. Two rows are safe to absorb, while one field-map override needs a human check before it becomes a hard rule.",
    collection: "Engine / Source Catalog",
    confidence: 0.64,
    contentScore: 0.58,
    sourcePrior: 0.06,
    state: "needs_you",
    reason: "New hard-routing rule needs approval",
    priority: "high",
    due: "Today 16:45",
    graph: ["Source Catalog", "Outlook", "Routing Rules"],
    tasks: [
      { title: "Approve Outlook field-map override", state: "open", due: "Today" },
    ],
    writebacks: [
      { actor: "Catalog agent", status: "Drafted rule change", at: "12:16" },
    ],
  },
];

const SOURCE_LABELS: Record<FilterKind, string> = {
  all: "All sources",
  gmail: "Gmail",
  drive: "Drive",
  notion: "Notion",
  linear: "Linear",
  outlook: "Outlook",
};

function percent(value: number): string {
  return `${Math.round(value * 100)}%`;
}

function stateLabel(state: IntakeState): string {
  if (state === "needs_you") return "Needs you";
  if (state === "review") return "Review";
  return "Routed";
}

function priorityLabel(priority: IntakeItem["priority"]): string {
  if (priority === "high") return "High";
  if (priority === "low") return "Low";
  return "Normal";
}

export function IntakeReview() {
  const [items, setItems] = useState<IntakeItem[]>(INITIAL_ITEMS);
  const [selectedId, setSelectedId] = useState(INITIAL_ITEMS[0]?.id ?? "");
  const [filter, setFilter] = useState<FilterKind>("all");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const timer = window.setTimeout(() => setLoading(false), 260);
    return () => window.clearTimeout(timer);
  }, []);

  const sourceCounts = useMemo(() => {
    const counts = new Map<FilterKind, number>([["all", items.length]]);
    for (const item of items) {
      counts.set(item.source, (counts.get(item.source) ?? 0) + 1);
    }
    return counts;
  }, [items]);

  const visibleItems = useMemo(
    () => items.filter((item) => filter === "all" || item.source === filter),
    [filter, items],
  );

  const selected = visibleItems.find((item) => item.id === selectedId) ?? visibleItems[0];

  useEffect(() => {
    if (visibleItems.length === 0) return;
    if (!visibleItems.some((item) => item.id === selectedId)) {
      setSelectedId(visibleItems[0].id);
    }
  }, [selectedId, visibleItems]);

  const updateSelected = (patch: Partial<IntakeItem>) => {
    if (!selected) return;
    setItems((current) =>
      current.map((item) =>
        item.id === selected.id
          ? {
              ...item,
              ...patch,
              writebacks: patch.writebacks ?? item.writebacks,
            }
          : item,
      ),
    );
  };

  const approveRoute = () => {
    if (!selected) return;
    updateSelected({
      state: "routed",
      confidence: Math.max(selected.confidence, 0.86),
      reason: "Approved route",
      writebacks: [
        { actor: "You", status: "Approved collection", at: "now" },
        ...selected.writebacks,
      ],
    });
  };

  const holdForReview = () => {
    if (!selected) return;
    updateSelected({
      state: "review",
      reason: "Held for source review",
      writebacks: [
        { actor: "You", status: "Moved to review", at: "now" },
        ...selected.writebacks,
      ],
    });
  };

  const addFollowUp = () => {
    if (!selected) return;
    const exists = selected.tasks.some((task) => task.title === "Review source route");
    updateSelected({
      tasks: exists
        ? selected.tasks
        : [
            { title: "Review source route", state: "open", due: "Today" },
            ...selected.tasks,
          ],
      writebacks: [
        { actor: "Task agent", status: "Added follow-up task", at: "now" },
        ...selected.writebacks,
      ],
    });
  };

  const retry = () => {
    setError(null);
    setLoading(true);
    window.setTimeout(() => setLoading(false), 220);
  };

  if (loading) {
    return (
      <section className="intake intake--loading" aria-label="Loading source intake">
        <div className="intake__masthead">
          <div>
            <div className="eyebrow skeleton skeleton--short" />
            <div className="skeleton skeleton--title" />
          </div>
          <div className="intake__status-strip">
            <div className="skeleton skeleton--metric" />
            <div className="skeleton skeleton--metric" />
            <div className="skeleton skeleton--metric" />
          </div>
        </div>
        <div className="intake__layout">
          <div className="skeleton skeleton--panel" />
          <div className="skeleton skeleton--panel skeleton--panel-wide" />
          <div className="skeleton skeleton--panel" />
        </div>
      </section>
    );
  }

  if (error) {
    return (
      <section className="intake intake--state" aria-label="Source intake error">
        <div className="intake__state-block">
          <SourceIcon size={28} />
          <h1>Source catalog unavailable</h1>
          <p>{error}</p>
          <button className="btn btn--primary" type="button" onClick={retry}>
            Retry catalog check
          </button>
        </div>
      </section>
    );
  }

  return (
    <section className="intake" aria-label="CommonPlace source intake">
      <div className="intake__masthead">
        <div className="intake__title-block">
          <div className="eyebrow">CommonPlace</div>
          <h1>Source Intake</h1>
          <p>
            {items.filter((item) => item.state === "needs_you").length} decisions need you,
            {" "}
            {items.filter((item) => item.state === "routed").length} already routed.
          </p>
        </div>
        <div className="intake__status-strip" aria-label="Intake summary">
          <div>
            <span>Records</span>
            <strong>{items.length}</strong>
          </div>
          <div>
            <span>Review</span>
            <strong>{items.filter((item) => item.state !== "routed").length}</strong>
          </div>
          <div>
            <span>Median confidence</span>
            <strong>69%</strong>
          </div>
        </div>
      </div>

      <div className="intake__layout">
        <aside className="source-panel" aria-label="Source filters">
          <div className="panel-title">
            <SourceIcon size={14} />
            Sources
          </div>
          <div className="source-filter-list">
            {(Object.keys(SOURCE_LABELS) as FilterKind[]).map((source) => (
              <button
                key={source}
                type="button"
                className={
                  "source-filter" + (filter === source ? " source-filter--active" : "")
                }
                onClick={() => setFilter(source)}
              >
                <span>{SOURCE_LABELS[source]}</span>
                <code>{sourceCounts.get(source) ?? 0}</code>
              </button>
            ))}
          </div>
          <div className="source-health">
            <div>
              <span>Sync window</span>
              <strong>08:30-12:16</strong>
            </div>
            <div>
              <span>Catalog</span>
              <strong>5 spokes</strong>
            </div>
          </div>
        </aside>

        <main className="review-workbench" aria-label="Review workbench">
          <div className="review-list" aria-label="Source records">
            {visibleItems.length === 0 ? (
              <div className="empty-state">
                <SourceIcon size={26} />
                <h2>No source records</h2>
                <p>{SOURCE_LABELS[filter]} has no records in this sync window.</p>
              </div>
            ) : (
              visibleItems.map((item, index) => (
                <button
                  key={item.id}
                  type="button"
                  style={{ "--i": index } as CSSProperties}
                  className={
                    "source-record" + (item.id === selected?.id ? " source-record--active" : "")
                  }
                  onClick={() => setSelectedId(item.id)}
                >
                  <span className={"source-record__state source-record__state--" + item.state}>
                    {stateLabel(item.state)}
                  </span>
                  <span className="source-record__title">{item.title}</span>
                  <span className="source-record__meta">
                    {item.sourceLabel} / {item.sourceContainer}
                  </span>
                  <span className="source-record__confidence">
                    <span style={{ width: percent(item.confidence) }} />
                  </span>
                </button>
              ))
            )}
          </div>

          {selected && (
            <article className="review-sheet" aria-label="Selected source record">
              <div className="review-sheet__head">
                <div>
                  <span className="source-pill">
                    {selected.sourceLabel} / {selected.externalId}
                  </span>
                  <h2>{selected.title}</h2>
                  <p>{selected.from} / fetched {selected.fetchedAt}</p>
                </div>
                <span className={"priority priority--" + selected.priority}>
                  {priorityLabel(selected.priority)}
                </span>
              </div>
              <p className="review-sheet__preview">{selected.preview}</p>
              <div className="review-sheet__body">{selected.body}</div>
              <div className="graph-strip" aria-label="Graph context">
                {selected.graph.map((node) => (
                  <span key={node}>{node}</span>
                ))}
              </div>
            </article>
          )}
        </main>

        {selected && (
          <aside className="routing-panel" aria-label="Routing decision">
            <div className="panel-title">
              <RouteIcon size={14} />
              Route
            </div>
            <div className="route-target">
              <span>Collection</span>
              <strong>{selected.collection}</strong>
            </div>
            <div className="confidence-meter">
              <div className="confidence-meter__top">
                <span>Confidence</span>
                <strong>{percent(selected.confidence)}</strong>
              </div>
              <div className="confidence-meter__bar">
                <span style={{ width: percent(selected.confidence) }} />
              </div>
              <div className="confidence-meter__parts">
                <span>content {percent(selected.contentScore)}</span>
                <span>source prior {percent(selected.sourcePrior)}</span>
              </div>
            </div>
            <p className="route-reason">{selected.reason}</p>
            <div className="decision-actions">
              <button type="button" className="btn btn--primary" onClick={approveRoute}>
                <CheckIcon size={14} />
                Approve
              </button>
              <button type="button" className="btn" onClick={holdForReview}>
                <QueueIcon size={14} />
                Review
              </button>
              <button type="button" className="btn" onClick={addFollowUp}>
                <TaskIcon size={14} />
                Task
              </button>
            </div>

            <div className="side-section">
              <div className="panel-title panel-title--small">
                <TaskIcon size={13} />
                Tasks
              </div>
              {selected.tasks.length ? (
                selected.tasks.map((task) => (
                  <div className="task-row" key={`${selected.id}-${task.title}`}>
                    <span>{task.title}</span>
                    <code>{task.due}</code>
                  </div>
                ))
              ) : (
                <div className="empty-inline">No task edges</div>
              )}
            </div>

            <div className="side-section">
              <div className="panel-title panel-title--small">Writebacks</div>
              {selected.writebacks.map((writeback) => (
                <div className="writeback-row" key={`${writeback.actor}-${writeback.status}-${writeback.at}`}>
                  <span>{writeback.status}</span>
                  <code>{writeback.actor} / {writeback.at}</code>
                </div>
              ))}
            </div>
          </aside>
        )}
      </div>
    </section>
  );
}
