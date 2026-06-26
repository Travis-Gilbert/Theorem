"use client";

import { cn } from "@/lib/utils";

export type OmnibarElevation = "ambient" | "active";

export function omnibarSurfaceClass(elevation: OmnibarElevation = "ambient", className?: string) {
  return cn(
    "pointer-events-auto w-full max-w-2xl overflow-hidden rounded-2xl border border-line bg-bg",
    elevation === "ambient" ? "elev-2" : "elev-3",
    className,
  );
}

export function omnibarRowClass(className?: string) {
  return cn("flex items-center gap-2 px-3 py-2", className);
}

export function omnibarIconButtonClass(active = false, className?: string) {
  return cn(
    "rounded-lg p-1.5 transition-colors",
    active ? "text-ox" : "text-muted-foreground hover:text-ink",
    className,
  );
}

export function omnibarSendButtonClass(className?: string) {
  return cn("rounded-lg bg-ox p-1.5 text-white hover:bg-ox-hover disabled:cursor-not-allowed disabled:opacity-45", className);
}
