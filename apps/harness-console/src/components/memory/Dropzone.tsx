"use client";

import * as React from "react";
import { useDropzone, type FileRejection } from "react-dropzone";
import { UploadCloud, FileText, FileArchive, FileType, CheckCircle2, Loader2, X } from "lucide-react";
import { harness, type Atom, type AtomKind } from "@/lib/harness";
import { toast } from "@/components/ui/toaster";
import { cn } from "@/lib/utils";

const ACCEPT = {
  "application/pdf": [".pdf"],
  "text/markdown": [".md", ".markdown"],
  "text/plain": [".txt"],
  "application/zip": [".zip"],
};

interface UploadItem {
  id: string;
  name: string;
  size: number;
  ext: string;
  progress: number; // 0..100
  status: "uploading" | "done" | "error";
}

function extOf(name: string): string {
  const m = name.toLowerCase().match(/\.([a-z0-9]+)$/);
  return m ? m[1] : "file";
}

function kindForExt(ext: string): AtomKind {
  if (ext === "pdf" || ext === "zip") return "source";
  return "note";
}

function fileIcon(ext: string) {
  if (ext === "pdf") return FileType;
  if (ext === "zip") return FileArchive;
  return FileText;
}

/**
 * The ingest dropzone: a persistent affordance card plus a drag-anywhere
 * overlay. Dropping a PDF / md / txt / zip simulates an upload with per-file
 * progress, then writes an atom per file through harness.saveAtom (optimistic
 * ingest) so the new atoms appear in the table on completion. The harness
 * auto-structures the real ingest server-side; here we model the per-file
 * progress and the resulting atoms.
 */
export function Dropzone({ onIngested }: { onIngested: () => void }) {
  const [items, setItems] = React.useState<UploadItem[]>([]);
  const timers = React.useRef<ReturnType<typeof setInterval>[]>([]);

  React.useEffect(() => () => timers.current.forEach(clearInterval), []);

  const ingestOne = React.useCallback(
    (file: File) => {
      const ext = extOf(file.name);
      const id = `up_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`;
      const item: UploadItem = {
        id,
        name: file.name,
        size: file.size,
        ext,
        progress: 0,
        status: "uploading",
      };
      setItems((prev) => [item, ...prev]);

      // Simulate streamed upload progress, then produce the atom on completion.
      const tick = setInterval(() => {
        setItems((prev) =>
          prev.map((it) => {
            if (it.id !== id || it.status !== "uploading") return it;
            const next = Math.min(100, it.progress + 8 + Math.random() * 14);
            return { ...it, progress: next };
          }),
        );
      }, 140);
      timers.current.push(tick);

      const finish = async () => {
        clearInterval(tick);
        const now = new Date().toISOString();
        const title = file.name.replace(/\.[a-z0-9]+$/i, "");
        const atom: Atom = {
          id: `atom_ingest_${id}`,
          title,
          kind: kindForExt(ext),
          summary: `Ingested from ${file.name} (${ext.toUpperCase()}, ${(file.size / 1024).toFixed(0)} KB).`,
          body: `# ${title}\n\nIngested from \`${file.name}\`. The harness auto-structured this drop into one or more atoms; this top-level atom carries the source and is vector-searchable.\n`,
          hydrated: true,
          contentPreview: `Ingested from ${file.name}.`,
          tags: ["ingest", ext],
          salience: 0.5,
          source: ext === "pdf" ? "pdf" : ext === "zip" ? "archive" : "upload",
          created: now,
          updated: now,
          lifecycle: "active",
          links: [],
          clusterId: undefined,
        };
        try {
          await harness.saveAtom(atom);
          setItems((prev) => prev.map((it) => (it.id === id ? { ...it, progress: 100, status: "done" } : it)));
          onIngested();
        } catch (e) {
          setItems((prev) => prev.map((it) => (it.id === id ? { ...it, status: "error" } : it)));
          toast.error("Ingest failed", { description: e instanceof Error ? e.message : String(e) });
        }
      };
      // Complete after the bar visually fills.
      const done = setTimeout(finish, 1500 + Math.random() * 600);
      timers.current.push(done as unknown as ReturnType<typeof setInterval>);
    },
    [onIngested],
  );

  const onDrop = React.useCallback(
    (accepted: File[], rejected: FileRejection[]) => {
      accepted.forEach(ingestOne);
      if (rejected.length) {
        toast.error("Unsupported file type", {
          description: `${rejected.length} file(s) skipped. Accepts PDF, Markdown, TXT, ZIP.`,
        });
      }
      if (accepted.length) {
        toast.success(`Ingesting ${accepted.length} file${accepted.length > 1 ? "s" : ""}`);
      }
    },
    [ingestOne],
  );

  const { getRootProps, getInputProps, isDragActive, open } = useDropzone({
    onDrop,
    accept: ACCEPT,
    noClick: true, // the card has an explicit Browse button instead
  });

  const dismiss = (id: string) => {
    setItems((prev) => prev.filter((it) => it.id !== id));
  };

  const activeCount = items.filter((it) => it.status === "uploading").length;

  return (
    <div {...getRootProps()} className="relative">
      <input {...getInputProps()} />

      {/* Drag-anywhere overlay: appears while a file is over the surface. */}
      {isDragActive && (
        <div className="fixed inset-0 z-[90] flex items-center justify-center backdrop-overlay">
          <div className="material-blueprint material flex flex-col items-center gap-3 px-10 py-12 elev-3">
            <div className="grid h-14 w-14 place-items-center rounded-full bg-[var(--ox-tint)] text-ox">
              <UploadCloud size={26} />
            </div>
            <p className="font-title text-subhead text-ink">Drop to ingest into memory</p>
            <p className="font-mono text-label text-muted-foreground">PDF, Markdown, TXT, ZIP</p>
          </div>
        </div>
      )}

      {/* Persistent dropzone affordance. */}
      <div
        className={cn(
          "material flex flex-col items-center gap-2 border-dashed px-6 py-6 text-center transition-colors",
          isDragActive && "border-[var(--ox)] bg-[var(--ox-tint)]",
        )}
      >
        <div className="grid h-10 w-10 place-items-center rounded-full bg-surface-2 text-muted-foreground">
          <UploadCloud size={18} />
        </div>
        <div>
          <p className="font-title text-body text-ink">Drop files to ingest into memory</p>
          <p className="mt-0.5 font-mono text-[11px] text-muted-foreground">
            PDF, Markdown, TXT, ZIP. Each file is auto-structured into atoms.
          </p>
        </div>
        <button
          type="button"
          onClick={open}
          className="mt-1 inline-flex h-8 items-center gap-2 rounded-md border border-line bg-bg px-3 font-mono text-label text-ink hover:bg-surface-2"
        >
          Browse files
        </button>
      </div>

      {/* Per-file progress list. */}
      {items.length > 0 && (
        <div className="mt-3 flex flex-col gap-1.5">
          {activeCount > 0 && (
            <div className="rail-group-label px-1">uploading {activeCount}</div>
          )}
          {items.map((it) => {
            const Icon = fileIcon(it.ext);
            return (
              <div key={it.id} className="flex items-center gap-3 rounded-md border border-line bg-surface px-3 py-2">
                <Icon size={16} className="shrink-0 text-muted-foreground" />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center justify-between gap-2">
                    <span className="truncate text-label text-ink" title={it.name}>
                      {it.name}
                    </span>
                    <span className="shrink-0 font-mono text-[11px] text-faint">
                      {it.status === "done"
                        ? "indexed"
                        : it.status === "error"
                          ? "error"
                          : `${Math.round(it.progress)}%`}
                    </span>
                  </div>
                  <div className="mt-1 h-1 w-full overflow-hidden rounded-full bg-surface-2">
                    <div
                      className={cn(
                        "h-full rounded-full transition-[width] duration-150",
                        it.status === "error" ? "bg-ox" : it.status === "done" ? "bg-live" : "bg-ox",
                      )}
                      style={{ width: `${it.progress}%` }}
                    />
                  </div>
                </div>
                {it.status === "uploading" ? (
                  <Loader2 size={14} className="animate-[spin_1s_linear_infinite] text-muted-foreground" />
                ) : it.status === "done" ? (
                  <CheckCircle2 size={14} className="text-live" />
                ) : (
                  <span className="text-[11px] text-ox">!</span>
                )}
                {it.status !== "uploading" && (
                  <button
                    onClick={() => dismiss(it.id)}
                    className="rounded p-0.5 text-faint hover:bg-surface-2 hover:text-ink"
                    title="Dismiss"
                  >
                    <X size={13} />
                  </button>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
