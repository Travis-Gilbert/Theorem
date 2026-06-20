import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** The shadcn `cn` helper: merge class lists, last Tailwind class wins. */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** Relative-time formatter for `updated`, `last used`, ledger timestamps. */
export function relativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const diff = Date.now() - then;
  const min = 60_000;
  const hr = 60 * min;
  const day = 24 * hr;
  if (diff < min) return "just now";
  if (diff < hr) return `${Math.floor(diff / min)}m ago`;
  if (diff < day) return `${Math.floor(diff / hr)}h ago`;
  if (diff < 30 * day) return `${Math.floor(diff / day)}d ago`;
  return new Date(iso).toLocaleDateString();
}

/** Truncate a key to a copy-safe prefix display, e.g. hk_live_abcd...wxyz. */
export function keyPrefix(key: string, head = 11, tail = 4): string {
  if (key.length <= head + tail) return key;
  return `${key.slice(0, head)}...${key.slice(-tail)}`;
}

/** Deterministic small hash for stable mock colors / cluster assignment. */
export function hashHue(seed: string): number {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return Math.abs(h) % 360;
}
