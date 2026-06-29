"use client";

import { useCallback, useRef, useState } from "react";

/** Trimmed file-upload hook (derived from the originui useFileUpload pattern),
 *  with per-file upload progress for the harness "add files" affordance. The
 *  progress here is simulated; swap `simulate` for a real XHR/fetch upload to
 *  the capability surface (POST /cap/ingest) when wiring the backend. */
export type UploadFile = {
  id: string;
  name: string;
  size: number;
  type: string;
  progress: number; // 0..100
  done: boolean;
};

const SIZES = ["B", "KB", "MB", "GB", "TB"];
export function formatBytes(bytes: number, decimals = 1): string {
  if (!bytes) return "0 B";
  const k = 1024;
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(decimals))} ${SIZES[i]}`;
}

export function useFileUpload(initial: UploadFile[] = []) {
  const [files, setFiles] = useState<UploadFile[]>(initial);
  const [isDragging, setDragging] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const timers = useRef<Record<string, ReturnType<typeof setInterval>>>({});

  const simulate = useCallback((id: string) => {
    timers.current[id] = setInterval(() => {
      setFiles((prev) =>
        prev.map((f) => {
          if (f.id !== id) return f;
          const next = Math.min(100, f.progress + Math.random() * 22 + 9);
          if (next >= 100) {
            clearInterval(timers.current[id]);
            delete timers.current[id];
            return { ...f, progress: 100, done: true };
          }
          return { ...f, progress: next };
        }),
      );
    }, 230);
  }, []);

  const addFiles = useCallback(
    (list: FileList | File[]) => {
      const arr = Array.from(list);
      if (!arr.length) return;
      const added: UploadFile[] = arr.map((file) => ({
        id: `${file.name}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        name: file.name,
        size: file.size,
        type: file.type,
        progress: 0,
        done: false,
      }));
      setFiles((prev) => [...added, ...prev]);
      added.forEach((f) => simulate(f.id));
    },
    [simulate],
  );

  // cosmetic add from a known name (e.g. dragged out of the DocTree) — no real File
  const addNamed = useCallback(
    (name: string, type: string, size: number) => {
      const f: UploadFile = {
        id: `${name}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        name,
        size,
        type,
        progress: 0,
        done: false,
      };
      setFiles((prev) => [f, ...prev]);
      simulate(f.id);
    },
    [simulate],
  );

  const removeFile = useCallback((id: string) => {
    if (timers.current[id]) {
      clearInterval(timers.current[id]);
      delete timers.current[id];
    }
    setFiles((prev) => prev.filter((f) => f.id !== id));
  }, []);

  const open = useCallback(() => inputRef.current?.click(), []);

  const onInputChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      if (e.target.files?.length) addFiles(e.target.files);
      if (inputRef.current) inputRef.current.value = "";
    },
    [addFiles],
  );

  const drag = {
    onDragEnter: useCallback((e: React.DragEvent) => {
      e.preventDefault();
      setDragging(true);
    }, []),
    onDragOver: useCallback((e: React.DragEvent) => {
      e.preventDefault();
    }, []),
    onDragLeave: useCallback((e: React.DragEvent) => {
      e.preventDefault();
      if (!e.currentTarget.contains(e.relatedTarget as Node)) setDragging(false);
    }, []),
    onDrop: useCallback(
      (e: React.DragEvent) => {
        e.preventDefault();
        setDragging(false);
        if (e.dataTransfer.files?.length) addFiles(e.dataTransfer.files);
      },
      [addFiles],
    ),
  };

  return { files, isDragging, inputRef, addFiles, addNamed, removeFile, open, onInputChange, drag };
}
