"use client";

import { Toaster as SonnerToaster } from "sonner";

/** Toasts retokenized to the console palette. */
export function Toaster() {
  return (
    <SonnerToaster
      position="bottom-right"
      toastOptions={{
        style: {
          background: "var(--bg)",
          color: "var(--ink)",
          border: "1px solid var(--line)",
          borderRadius: "8px",
          fontFamily: "var(--font-plex-mono), monospace",
          fontSize: "13px",
        },
      }}
    />
  );
}

export { toast } from "sonner";
