"use client";

import * as React from "react";
import { Archive, Trash2, RotateCcw, Save, Link2, Hash, X } from "lucide-react";
import { harness, type Atom, type AtomKind } from "@/lib/harness";
import { Sheet, SheetContent, SheetTitle, SheetDescription } from "@/components/ui/sheet";
import { MarkdownEditor } from "@/components/editor/MarkdownEditor";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input, Textarea } from "@/components/ui/input";
import { Separator } from "@/components/ui/misc";
import { toast } from "@/components/ui/toaster";
import { cn, relativeTime } from "@/lib/utils";

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

interface EditorProps {
  atom: Atom | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Called after a save / archive / trash / restore so the page reloads. */
  onMutated: () => void;
  /** Open a wikilink target by title (the page resolves it to an atom). */
  onOpenLink?: (title: string) => void;
}

/**
 * The atom editor side panel, shared by every memory mode (table row click and
 * graph node click open the same panel). The Sheet shell stays mounted; the
 * stateful form remounts per atom (keyed by id) so the draft re-seeds without a
 * synchronizing effect.
 */
export function AtomEditor({ atom, open, onOpenChange, onMutated, onOpenLink }: EditorProps) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="gap-0 p-0">
        {atom && (
          <AtomEditorForm
            key={`${atom.id}:${atom.hydrated ? "hydrated" : "reference"}`}
            atom={atom}
            onOpenChange={onOpenChange}
            onMutated={onMutated}
            onOpenLink={onOpenLink}
          />
        )}
      </SheetContent>
    </Sheet>
  );
}

/**
 * The stateful editor body. A markdown body over the calm CodeMirror surface,
 * editable title/kind/summary/tags, wikilinked relations, a read-only metadata
 * block, and the lifecycle actions (archive / trash with a reason / restore).
 * Saving routes through harness.saveAtom (self_revise / upsert_note), which
 * produces a revision rather than overwriting.
 */
function AtomEditorForm({
  atom,
  onOpenChange,
  onMutated,
  onOpenLink,
}: {
  atom: Atom;
  onOpenChange: (open: boolean) => void;
  onMutated: () => void;
  onOpenLink?: (title: string) => void;
}) {
  // Local draft, initialized from the atom this form was keyed to.
  const [draft, setDraft] = React.useState<Atom>(atom);
  const [tagInput, setTagInput] = React.useState("");
  const [saving, setSaving] = React.useState(false);
  const [trashing, setTrashing] = React.useState(false);
  const [trashReason, setTrashReason] = React.useState("");

  const dirty = JSON.stringify(draft) !== JSON.stringify(atom);
  const archived = draft.lifecycle === "archived";
  const trashed = draft.lifecycle === "trash";
  const restorable = archived || trashed;

  // Render the body's [[wikilinks]] into chips that reopen the editor.
  const wikilinks = React.useMemo(() => {
    const found = new Set<string>(draft.links);
    const re = /\[\[([^\]]+)\]\]/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(draft.body)) !== null) found.add(m[1].trim());
    return Array.from(found).filter(Boolean);
  }, [draft.body, draft.links]);

  const set = <K extends keyof Atom>(key: K, value: Atom[K]) =>
    setDraft((d) => (d ? { ...d, [key]: value } : d));

  const addTag = () => {
    const t = tagInput.trim().replace(/^#/, "");
    if (!t || draft.tags.includes(t)) return setTagInput("");
    set("tags", [...draft.tags, t]);
    setTagInput("");
  };
  const removeTag = (t: string) => set("tags", draft.tags.filter((x) => x !== t));

  const onSave = async () => {
    setSaving(true);
    try {
      const saved = await harness.saveAtom(draft);
      setDraft(saved);
      toast.success("Revision saved", { description: `${saved.title} updated ${relativeTime(saved.updated)}.` });
      onMutated();
    } catch (e) {
      toast.error("Save failed", { description: e instanceof Error ? e.message : String(e) });
    } finally {
      setSaving(false);
    }
  };

  const onArchive = async () => {
    try {
      await harness.archiveAtom(draft.id);
      toast.success("Archived", { description: "Removed from Active. Restore it from the Archived view." });
      onMutated();
      onOpenChange(false);
    } catch (e) {
      toast.error("Archive failed", { description: e instanceof Error ? e.message : String(e) });
    }
  };

  const onTrash = async () => {
    try {
      await harness.trashAtom(draft.id, trashReason.trim() || "no reason given");
      toast.success("Moved to Trash", { description: "Restorable from the Trash view until purge." });
      onMutated();
      onOpenChange(false);
    } catch (e) {
      toast.error("Trash failed", { description: e instanceof Error ? e.message : String(e) });
    }
  };

  const onRestore = async () => {
    try {
      await harness.restoreAtom(draft.id);
      toast.success("Restored to Active");
      onMutated();
      onOpenChange(false);
    } catch (e) {
      toast.error("Restore failed", { description: e instanceof Error ? e.message : String(e) });
    }
  };

  return (
    <>
        {/* Header: kind + title, editable. */}
        <div className="flex flex-col gap-3 border-b border-line px-6 pb-4 pt-6">
          <div className="flex items-center gap-2 pr-8">
            <select
              value={draft.kind}
              onChange={(e) => set("kind", e.target.value as AtomKind)}
              className="h-7 rounded border border-line bg-bg px-2 font-mono text-[11px] text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--ox-ring)]"
              aria-label="Atom kind"
            >
              {KIND_OPTIONS.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
            {(archived || trashed) && (
              <Badge tone={trashed ? "warn" : "neutral"}>{draft.lifecycle}</Badge>
            )}
            <span className="ml-auto font-mono text-[11px] text-faint">
              updated {relativeTime(draft.updated)}
            </span>
          </div>
          <SheetTitle asChild>
            <input
              value={draft.title}
              onChange={(e) => set("title", e.target.value)}
              className="w-full bg-transparent font-title text-title text-ink outline-none placeholder:text-faint"
              placeholder="Atom title"
            />
          </SheetTitle>
          <SheetDescription asChild>
            <Textarea
              value={draft.summary}
              onChange={(e) => set("summary", e.target.value)}
              rows={2}
              placeholder="One-line summary"
              className="resize-none border-line bg-surface text-label text-muted-foreground"
            />
          </SheetDescription>
        </div>

        {/* Scrolling body. */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          {/* Tags */}
          <div className="mb-4">
            <div className="rail-group-label mb-2 flex items-center gap-1">
              <Hash size={11} /> tags
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              {draft.tags.map((t) => (
                <button
                  key={t}
                  onClick={() => removeTag(t)}
                  className="group inline-flex items-center gap-1 rounded border border-line bg-surface-2 px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground hover:border-[var(--ox)] hover:text-ox"
                  title="Remove tag"
                >
                  {t}
                  <X size={10} className="opacity-50 group-hover:opacity-100" />
                </button>
              ))}
              <Input
                value={tagInput}
                onChange={(e) => setTagInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === ",") {
                    e.preventDefault();
                    addTag();
                  }
                }}
                onBlur={addTag}
                placeholder="add tag"
                className="h-7 w-24 border-dashed font-mono text-[11px]"
              />
            </div>
          </div>

          {/* Markdown body */}
          <div className="mb-4">
            <div className="rail-group-label mb-2">body</div>
            <MarkdownEditor
              value={draft.body}
              onChange={(v) => set("body", v)}
              language="markdown"
              minHeight="260px"
            />
          </div>

          {/* Wikilinks */}
          <div className="mb-4">
            <div className="rail-group-label mb-2 flex items-center gap-1">
              <Link2 size={11} /> links
            </div>
            {wikilinks.length ? (
              <div className="flex flex-wrap gap-1.5">
                {wikilinks.map((l) => (
                  <button
                    key={l}
                    onClick={() => onOpenLink?.(l)}
                    className="inline-flex items-center gap-1 rounded border border-line bg-[var(--ox-tint)] px-2 py-0.5 font-mono text-[11px] text-ox hover:underline"
                    title="Open linked atom"
                  >
                    [[{l}]]
                  </button>
                ))}
              </div>
            ) : (
              <p className="font-mono text-[11px] text-faint">
                no relations. add [[wikilinks]] in the body to link atoms.
              </p>
            )}
          </div>

          <Separator className="my-4" />

          {/* Read-only metadata */}
          <div className="rail-group-label mb-2">metadata</div>
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 font-mono text-[11px]">
            <Meta label="id" value={draft.id} />
            <Meta label="created" value={`${relativeTime(draft.created)} (${new Date(draft.created).toLocaleString()})`} />
            <Meta label="salience" value={draft.salience.toFixed(2)} />
            {typeof draft.fitness === "number" && <Meta label="fitness" value={draft.fitness.toFixed(2)} />}
            {draft.source && <Meta label="source" value={draft.source} />}
            <Meta label="cluster" value={draft.clusterId ?? "unclustered"} />
          </dl>
        </div>

        {/* Footer actions: one primary (Save), neutral lifecycle controls. */}
        <div className="border-t border-line px-6 py-3">
          {trashing ? (
            <div className="flex flex-col gap-2">
              <Input
                autoFocus
                value={trashReason}
                onChange={(e) => setTrashReason(e.target.value)}
                placeholder="Reason for trashing (recorded with forget)"
                className="text-label"
              />
              <div className="flex items-center justify-end gap-2">
                <Button variant="ghost" size="sm" onClick={() => setTrashing(false)}>
                  Cancel
                </Button>
                <Button variant="danger" size="sm" onClick={onTrash}>
                  <Trash2 size={14} /> Confirm trash
                </Button>
              </div>
            </div>
          ) : (
            <div className="flex items-center gap-2">
              {restorable ? (
                <Button variant="primary" size="sm" onClick={onRestore}>
                  <RotateCcw size={14} /> Restore to Active
                </Button>
              ) : (
                <Button variant="primary" size="sm" onClick={onSave} disabled={!dirty || saving}>
                  <Save size={14} /> {saving ? "Saving..." : dirty ? "Save revision" : "Saved"}
                </Button>
              )}
              <div className="ml-auto flex items-center gap-1">
                {!archived && !trashed && (
                  <Button variant="ghost" size="sm" onClick={onArchive} title="Archive (self_archive)">
                    <Archive size={14} /> Archive
                  </Button>
                )}
                {!trashed && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setTrashing(true)}
                    className="text-ox hover:bg-[var(--ox-tint)]"
                    title="Trash (forget)"
                  >
                    <Trash2 size={14} /> Trash
                  </Button>
                )}
              </div>
            </div>
          )}
        </div>
    </>
  );
}

function Meta({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt className="text-faint">{label}</dt>
      <dd className={cn("truncate text-muted-foreground")} title={value}>
        {value}
      </dd>
    </>
  );
}
