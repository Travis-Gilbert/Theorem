"use client";

import * as React from "react";
import {
  AlertTriangle,
  CheckCircle2,
  CircleDot,
  Hash,
  Loader2,
  Play,
  Upload,
  Users,
} from "lucide-react";
import { cn, relativeTime } from "@/lib/utils";
import { harness, type Skill, type SkillFile, type SkillStatus, type SkillUseReceipt } from "@/lib/harness";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input, Textarea } from "@/components/ui/input";
import { Label } from "@/components/ui/misc";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { toast } from "@/components/ui/toaster";
import { RetroFrame } from "@/components/retro/retro";
import { MarkdownEditor, type EditorLanguage } from "@/components/editor/MarkdownEditor";
import { CollaborativeEditor } from "@/components/ide/CollaborativeEditor";
import { FileTree } from "./FileTree";

/* ----------------------------------------------------------------------------
 * Skill metadata vocabulary (shared with SkillList).
 * ------------------------------------------------------------------------- */

export const SKILL_STATUSES: SkillStatus[] = [
  "draft",
  "shadow",
  "advisory",
  "validated",
  "canonical",
  "retired",
];

export function statusTone(status: SkillStatus): "neutral" | "accent" | "live" | "warn" | "ink" {
  switch (status) {
    case "canonical":
      return "ink";
    case "validated":
      return "live";
    case "advisory":
    case "shadow":
      return "accent";
    case "retired":
      return "warn";
    case "draft":
    default:
      return "neutral";
  }
}

/** Map a file extension to the editor language the MarkdownEditor understands. */
export function languageForPath(path: string): EditorLanguage {
  const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
  switch (ext) {
    case "md":
    case "markdown":
      return "markdown";
    case "ts":
    case "tsx":
      return "typescript";
    case "js":
    case "jsx":
    case "mjs":
    case "cjs":
      return "javascript";
    case "rs":
      return "rust";
    default:
      return "text";
  }
}

function fileLanguage(path: string): SkillFile["language"] {
  const lang = languageForPath(path);
  return lang === "javascript" ? "typescript" : lang === "rust" ? "rust" : lang;
}

/* ----------------------------------------------------------------------------
 * SKILL.md frontmatter validation.
 * ------------------------------------------------------------------------- */

export interface SkillValidation {
  ok: boolean;
  name?: string;
  description?: string;
  errors: string[];
}

/** Parse and validate SKILL.md frontmatter (name + description are required). */
export function validateSkillMd(content: string): SkillValidation {
  const errors: string[] = [];
  const match = content.match(/^---\s*\n([\s\S]*?)\n---/);
  if (!match) {
    return { ok: false, errors: ["SKILL.md is missing YAML frontmatter (--- name / description ---)."] };
  }
  const front = match[1];
  const read = (key: string): string | undefined => {
    const line = front.match(new RegExp(`^${key}:\\s*(.+)$`, "m"));
    return line ? line[1].trim().replace(/^["']|["']$/g, "") : undefined;
  };
  const name = read("name");
  const description = read("description");
  if (!name) errors.push("Frontmatter is missing a `name`.");
  if (!description) errors.push("Frontmatter is missing a `description` (what the skill does).");
  return { ok: errors.length === 0, name, description, errors };
}

/** Deterministic content hash preview so editing visibly bumps the address. */
function previewHash(files: SkillFile[]): string {
  let h = 2166136261;
  const seed = files.map((f) => `${f.path}:${f.content}`).join("");
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return `sha256:${(h >>> 0).toString(16).padStart(8, "0")}`;
}

/* ----------------------------------------------------------------------------
 * SkillEditor
 * ------------------------------------------------------------------------- */

export function SkillEditor({
  skill,
  onChange,
  onPublished,
}: {
  skill: Skill;
  /** Local edits propagate up so the list can show a dirty indicator. */
  onChange: (next: Skill) => void;
  /** Fired with the server skill after a successful publish (new hash). */
  onPublished: (published: Skill) => void;
}) {
  const [activePath, setActivePath] = React.useState<string>(skill.files[0]?.path ?? "SKILL.md");
  const [mode, setMode] = React.useState<"plain" | "collab">("plain");
  const [publishing, setPublishing] = React.useState(false);
  const [receipt, setReceipt] = React.useState<SkillUseReceipt | null>(null);
  const [applying, setApplying] = React.useState(false);

  // No reset effect needed: the parent (skills/page.tsx) renders SkillEditor with
  // key={selected.id}, so switching packs remounts this component and the useState
  // initializers above re-derive activePath / mode / receipt for the new skill.
  // (This also avoids the per-keystroke active-file reset that a skill.files-keyed
  // effect caused.)

  const activeFile = skill.files.find((f) => f.path === activePath) ?? skill.files[0];
  const skillMd = skill.files.find((f) => f.path === "SKILL.md");
  const validation = React.useMemo(
    () => validateSkillMd(skillMd?.content ?? ""),
    [skillMd?.content],
  );
  const localHash = React.useMemo(() => previewHash(skill.files), [skill.files]);
  const dirty = localHash !== skill.contentHash;

  /* file content edits ---------------------------------------------------- */
  const updateFileContent = (content: string) => {
    if (!activeFile) return;
    onChange({
      ...skill,
      files: skill.files.map((f) => (f.path === activeFile.path ? { ...f, content } : f)),
    });
  };

  const addFile = (path: string): boolean => {
    if (skill.files.some((f) => f.path === path)) return false;
    const file: SkillFile = { path, language: fileLanguage(path), content: "" };
    onChange({ ...skill, files: [...skill.files, file] });
    setActivePath(path);
    return true;
  };

  const deleteFile = (path: string) => {
    if (path === "SKILL.md") return;
    const next = skill.files.filter((f) => f.path !== path);
    onChange({ ...skill, files: next });
    if (activePath === path) setActivePath("SKILL.md");
  };

  /* metadata edits -------------------------------------------------------- */
  const setName = (name: string) => onChange({ ...skill, name });
  const setDescription = (description: string) => onChange({ ...skill, description });
  const setStatus = (status: SkillStatus) => onChange({ ...skill, status });

  /* publish --------------------------------------------------------------- */
  const publish = async () => {
    if (!validation.ok) {
      toast.error("Fix SKILL.md before publishing", {
        description: validation.errors[0],
      });
      return;
    }
    setPublishing(true);
    try {
      const published = await harness.publishSkill({
        ...skill,
        // Frontmatter is authoritative for the published identity.
        name: validation.name ?? skill.name,
        description: validation.description ?? skill.description,
      });
      onPublished(published);
      toast.success(`Published ${published.name}`, {
        description: `Content hash ${published.contentHash}`,
      });
    } catch (e) {
      toast.error("Publish failed", { description: e instanceof Error ? e.message : String(e) });
    } finally {
      setPublishing(false);
    }
  };

  /* apply (test-run) ------------------------------------------------------ */
  const apply = async () => {
    setApplying(true);
    setReceipt(null);
    try {
      const result = await harness.applySkill(skill.id);
      setReceipt(result);
      if (result.ok) toast.success(`Applied ${skill.name}`);
      else toast.error(`Apply reported a failure`, { description: result.summary });
    } catch (e) {
      toast.error("Apply failed", { description: e instanceof Error ? e.message : String(e) });
    } finally {
      setApplying(false);
    }
  };

  const editorLanguage = activeFile ? languageForPath(activeFile.path) : "markdown";

  return (
    <div className="flex h-full min-h-0 flex-col gap-4">
      {/* header row: identity + publish ------------------------------------ */}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h3 className="truncate font-title text-subhead text-ink">{skill.name || "untitled-skill"}</h3>
            <Badge tone={statusTone(skill.status)}>{skill.status}</Badge>
            {dirty && (
              <span className="inline-flex items-center gap-1 font-mono text-[11px] text-warn">
                <CircleDot size={11} /> unpublished
              </span>
            )}
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[11px] text-muted-foreground">
            <span className="inline-flex items-center gap-1">
              <Hash size={11} />
              {dirty ? localHash : skill.contentHash}
            </span>
            <span>{skill.version}</span>
            <span>updated {relativeTime(skill.updated)}</span>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button variant="outline" size="sm" onClick={apply} disabled={applying}>
            {applying ? <Loader2 size={14} className="animate-spin" /> : <Play size={14} />}
            apply
          </Button>
          <Button variant="primary" size="sm" onClick={publish} disabled={publishing || !validation.ok}>
            {publishing ? <Loader2 size={14} className="animate-spin" /> : <Upload size={14} />}
            {publishing ? "publishing" : "publish"}
          </Button>
        </div>
      </div>

      {/* validation banner ------------------------------------------------- */}
      {validation.ok ? (
        <div className="flex items-center gap-2 rounded-md border border-[var(--live)] bg-surface px-3 py-2 font-mono text-label text-[var(--live)]">
          <CheckCircle2 size={14} className="shrink-0" />
          SKILL.md valid: name and description present in frontmatter.
        </div>
      ) : (
        <div className="rounded-md border border-ox bg-[var(--ox-tint)] px-3 py-2">
          <div className="flex items-center gap-2 font-mono text-label text-ox">
            <AlertTriangle size={14} className="shrink-0" />
            SKILL.md is malformed
          </div>
          <ul className="mt-1 list-disc pl-7 font-mono text-[11px] text-ox">
            {validation.errors.map((err) => (
              <li key={err}>{err}</li>
            ))}
          </ul>
        </div>
      )}

      {/* editor body: tree | editor | metadata ----------------------------- */}
      <div className="grid min-h-0 flex-1 gap-4 lg:grid-cols-[200px_1fr_240px]">
        {/* file tree */}
        <div className="material min-h-[280px] p-2 lg:min-h-0">
          <FileTree
            files={skill.files}
            activePath={activePath}
            onSelect={setActivePath}
            onAddFile={addFile}
            onDeleteFile={deleteFile}
          />
        </div>

        {/* editor surface, wrapped in a RetroFrame instrument */}
        <div className="flex min-w-0 flex-col gap-2">
          <div className="flex items-center justify-between gap-2">
            <span className="truncate font-mono text-label text-muted-foreground">{activeFile?.path}</span>
            <div className="inline-flex items-center rounded-md border border-line bg-surface p-0.5">
              <button
                type="button"
                onClick={() => setMode("plain")}
                className={cn(
                  "rounded px-2 py-1 font-mono text-[11px] transition-colors",
                  mode === "plain" ? "bg-bg text-ink shadow-elev-1" : "text-muted-foreground hover:text-ink",
                )}
              >
                editor
              </button>
              <button
                type="button"
                onClick={() => setMode("collab")}
                className={cn(
                  "inline-flex items-center gap-1 rounded px-2 py-1 font-mono text-[11px] transition-colors",
                  mode === "collab" ? "bg-bg text-ink shadow-elev-1" : "text-muted-foreground hover:text-ink",
                )}
              >
                <Users size={11} /> co-edit
              </button>
            </div>
          </div>

          <RetroFrame className="min-h-0 flex-1 p-2">
            {activeFile ? (
              mode === "plain" ? (
                <MarkdownEditor
                  key={activeFile.path}
                  value={activeFile.content}
                  language={editorLanguage}
                  onChange={updateFileContent}
                  minHeight="340px"
                />
              ) : (
                <CollaborativeEditor
                  key={`collab-${activeFile.path}`}
                  initialDoc={activeFile.content}
                  language={editorLanguage}
                  agentSnippet={"\n# agent: tightened the skill steps and added a receipt note\n"}
                  minHeight="340px"
                />
              )
            ) : (
              <div className="grid h-full place-items-center font-mono text-label text-muted-foreground">
                No file selected.
              </div>
            )}
          </RetroFrame>

          {mode === "collab" && (
            <p className="font-mono text-[11px] text-muted-foreground">
              Human and harnessed agent edit the same Yjs document live. Switch back to the editor to save
              the file content for publish.
            </p>
          )}
        </div>

        {/* metadata panel */}
        <div className="material flex flex-col gap-4 p-4">
          <span className="rail-group-label">metadata</span>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="skill-name">name</Label>
            <Input
              id="skill-name"
              value={skill.name}
              className="font-mono text-label"
              onChange={(e) => setName(e.target.value)}
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="skill-desc">what does this skill do</Label>
            <Textarea
              id="skill-desc"
              rows={4}
              value={skill.description}
              className="text-label"
              onChange={(e) => setDescription(e.target.value)}
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="skill-status">status</Label>
            <Select value={skill.status} onValueChange={(v) => setStatus(v as SkillStatus)}>
              <SelectTrigger id="skill-status" aria-label="Skill status">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {SKILL_STATUSES.map((s) => (
                  <SelectItem key={s} value={s}>
                    {s}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="font-mono text-[11px] text-faint">
              Promote up the ladder or retire. Lifecycle changes save on the next publish.
            </p>
          </div>

          {/* use receipt */}
          {receipt && (
            <div className="rounded-md border border-line bg-surface p-3">
              <div className="flex items-center gap-2 font-mono text-label">
                {receipt.ok ? (
                  <CheckCircle2 size={13} className="text-[var(--live)]" />
                ) : (
                  <AlertTriangle size={13} className="text-ox" />
                )}
                <span className="text-ink">use receipt</span>
                <span className="ml-auto text-[11px] text-faint">{relativeTime(receipt.appliedAt)}</span>
              </div>
              <p className="mt-1 text-[11px] text-muted-foreground">{receipt.summary}</p>
              <ol className="mt-2 space-y-0.5 font-mono text-[11px] text-muted-foreground">
                {receipt.steps.map((step, i) => (
                  <li key={step} className="flex gap-2">
                    <span className="text-faint">{String(i + 1).padStart(2, "0")}</span>
                    <span>{step}</span>
                  </li>
                ))}
              </ol>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
