"use client";

import { useMemo, useRef, useState } from "react";
import { animate } from "animejs";
import { Database, Mail, Search, UploadCloud, X, Check } from "lucide-react";
import { useFileUpload, formatBytes, type UploadFile } from "@/lib/use-file-upload";
import { DOC_DND_MIME } from "@/components/marketing/DocTree";

/** Sidebar end-cap with two modes, toggled by the database / mail icons in the
 *  header. UPLOAD: labeled drop zone + per-file progress + files search (default).
 *  EMAIL: the drop zone becomes an early-access capture. Token-styled for the
 *  parchment canvas. */

const SEED: UploadFile[] = [
  { id: "s1", name: "auth-flow.md", size: 8240, type: "text/markdown", progress: 100, done: true },
  { id: "s2", name: "architecture.png", size: 182873, type: "image/png", progress: 100, done: true },
];

// extension -> a-file-icon name (MIT, Mallowigi), served from /public/file-icons
const ICON_BY_EXT: Record<string, string> = {
  md: "markdown", markdown: "markdown",
  ts: "typescript", tsx: "react", jsx: "react",
  js: "js", mjs: "js", cjs: "js",
  py: "python", rs: "rust", go: "go",
  json: "json", toml: "toml", yaml: "yaml", yml: "yaml",
  css: "css", html: "html", htm: "html", pdf: "pdf",
  mp4: "video", mov: "video", webm: "video", mkv: "video",
};
function iconName(name: string, type: string) {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (type.startsWith("image/") || ["png", "jpg", "jpeg", "gif", "webp", "svg", "avif"].includes(ext))
    return "image";
  return ICON_BY_EXT[ext] ?? "file";
}

export function UploadDock() {
  const { files, isDragging, inputRef, addNamed, removeFile, open, onInputChange, drag } = useFileUpload(SEED);
  const [mode, setMode] = useState<"upload" | "email">("upload");
  const [q, setQ] = useState("");
  const [email, setEmail] = useState("");
  const [submitted, setSubmitted] = useState(false);
  const dropRef = useRef<HTMLButtonElement>(null);

  // accept a doc dragged out of the DocTree (cosmetic): add it + pulse the dock
  const onDrop = (e: React.DragEvent) => {
    const payload = e.dataTransfer.getData(DOC_DND_MIME);
    drag.onDrop(e); // clears the drag highlight; ignores empty file list
    if (!payload) return;
    try {
      const d = JSON.parse(payload) as { name: string; type: string; size: number };
      addNamed(d.name, d.type, d.size);
    } catch {
      return;
    }
    if (dropRef.current && !window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      animate(dropRef.current, { scale: [1, 1.05, 1], duration: 420, ease: "inOutSine" });
    }
  };

  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return needle ? files.filter((f) => f.name.toLowerCase().includes(needle)) : files;
  }, [files, q]);

  return (
    <section className="rounded-2xl border border-line p-3">
      {/* header — the database / mail toggle mirrors each other */}
      <div className="mb-2.5 flex items-center gap-2 px-1">
        <div className="flex items-center gap-1">
          <ModeIcon active={mode === "upload"} onClick={() => setMode("upload")} label="Add files">
            <Database size={13} />
          </ModeIcon>
          <ModeIcon active={mode === "email"} onClick={() => setMode("email")} label="Get early access">
            <Mail size={13} />
          </ModeIcon>
        </div>
        <span className="flex-1 font-mono text-[10px] uppercase tracking-[0.12em] text-muted-foreground">
          {mode === "upload" ? "Add to your harness" : "Get your harness"}
        </span>
        {mode === "upload" && (
          <span className="font-mono text-[10px] text-muted-foreground">{files.length}</span>
        )}
      </div>

      {mode === "upload" ? (
        <>
          {/* search */}
          <div
            className="mb-2.5 flex items-center gap-2 rounded-lg border border-line px-2.5"
            style={{ background: "var(--recess)" }}
          >
            <Search size={13} className="text-muted-foreground" />
            <input
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder="Search files…"
              aria-label="Search files"
              className="w-full bg-transparent py-2 text-[12px] text-ink outline-none placeholder:text-muted-foreground"
            />
          </div>

          {/* drop zone */}
          <button
            ref={dropRef}
            type="button"
            onClick={open}
            onDrop={onDrop}
            onDragOver={drag.onDragOver}
            onDragEnter={drag.onDragEnter}
            onDragLeave={drag.onDragLeave}
            className="flex w-full flex-col items-center gap-1.5 rounded-xl border border-dashed px-3 py-4 text-center transition-colors"
            style={{
              borderColor: isDragging ? "var(--ox)" : "var(--line-strong)",
              background: isDragging ? "var(--ox-tint)" : "transparent",
            }}
          >
            <UploadCloud size={20} style={{ color: isDragging ? "var(--ox)" : "var(--muted)" }} />
            <span className="text-[12.5px] font-medium text-ink">Drop files or click to add</span>
            <span className="text-[10.5px] text-muted-foreground">
              code, docs, images — straight into your graph
            </span>
          </button>
          <input ref={inputRef} type="file" multiple onChange={onInputChange} className="sr-only" />

          {/* file list with progress */}
          {shown.length > 0 && (
            <ul className="mt-2.5 flex max-h-[176px] flex-col gap-1 overflow-y-auto pr-0.5">
              {shown.map((f) => {
                return (
                  <li key={f.id} className="group flex items-center gap-2.5 rounded-lg bg-black/[.03] px-2 py-1.5">
                    {/* eslint-disable-next-line @next/next/no-img-element */}
                    <img src={`/file-icons/${iconName(f.name, f.type)}.svg`} alt="" className="h-4 w-4 flex-none" />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center justify-between gap-2">
                        <span className="truncate text-[12px] text-ink">{f.name}</span>
                        <span className="flex-none font-mono text-[10px] text-muted-foreground">
                          {f.done ? formatBytes(f.size) : `${Math.round(f.progress)}%`}
                        </span>
                      </div>
                      <div className="mt-1 h-1 overflow-hidden rounded-full" style={{ background: "rgba(42,36,32,.1)" }}>
                        <div
                          className="h-full rounded-full transition-[width] duration-200"
                          style={{ width: `${f.progress}%`, background: f.done ? "var(--green)" : "var(--ox)" }}
                        />
                      </div>
                    </div>
                    {f.done ? <Check size={13} className="flex-none" style={{ color: "var(--green)" }} /> : null}
                    <button
                      type="button"
                      onClick={() => removeFile(f.id)}
                      aria-label={`Remove ${f.name}`}
                      className="grid h-5 w-5 flex-none place-items-center rounded text-muted-foreground opacity-0 transition-opacity hover:text-ink group-hover:opacity-100"
                    >
                      <X size={12} />
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </>
      ) : (
        /* EMAIL CAPTURE — the uploader's other state */
        <div className="rounded-xl border border-dashed px-3 py-4" style={{ borderColor: "var(--line-strong)" }}>
          {submitted ? (
            <div className="flex flex-col items-center gap-1.5 py-1 text-center">
              <span className="grid h-9 w-9 place-items-center rounded-full" style={{ background: "var(--ox-tint)", color: "var(--ox)" }}>
                <Check size={18} />
              </span>
              <span className="text-[13px] font-medium text-ink">You&apos;re on the list.</span>
              <span className="text-[10.5px] text-muted-foreground">We&apos;ll send your harness key soon.</span>
            </div>
          ) : (
            <form
              onSubmit={(e) => {
                e.preventDefault();
                if (email.includes("@")) setSubmitted(true);
              }}
              className="flex flex-col gap-2"
            >
              <p className="text-center text-[11px] text-muted-foreground">
                Start free — <span className="text-ink">10,000 calls</span>, no card.
              </p>
              <label className="mk-field flex items-center rounded-lg px-3">
                <input
                  type="email"
                  required
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  placeholder="you@workshop.dev"
                  aria-label="Email address"
                  className="w-full bg-transparent py-2 text-[12.5px] text-ink outline-none placeholder:text-muted-foreground"
                />
              </label>
              <button type="submit" className="mk-cta w-full rounded-lg py-2 text-[13px] font-semibold">
                Get early access
              </button>
            </form>
          )}
        </div>
      )}
    </section>
  );
}

function ModeIcon({
  active,
  onClick,
  label,
  children,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      aria-pressed={active}
      title={label}
      className="grid h-6 w-6 place-items-center rounded-md transition-colors"
      style={{
        background: active ? "var(--ox-tint)" : "transparent",
        color: active ? "var(--ox)" : "var(--muted)",
      }}
    >
      {children}
    </button>
  );
}
