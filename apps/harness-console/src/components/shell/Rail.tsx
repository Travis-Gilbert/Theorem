"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import {
  Bot,
  Brain,
  Boxes,
  Radio,
  History,
  KeyRound,
  Plug,
  Gauge,
  Settings as SettingsIcon,
  Frame,
  Inbox,
  ChevronDown,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { UsagePulse } from "./UsagePulse";

interface NavItem {
  href: string;
  label: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
}

// Top-level daily surfaces: the persistent spatial home and the action queue.
const TOP: NavItem[] = [
  { href: "/canvas", label: "Canvas", icon: Frame },
  { href: "/inbox", label: "Inbox", icon: Inbox },
];

const GROUPS: { label: string; items: NavItem[] }[] = [
  {
    // The composed agent and everything you program/observe about it.
    label: "Agents",
    items: [
      { href: "/agent", label: "Agent", icon: Bot },
      { href: "/memory", label: "Memory", icon: Brain },
      { href: "/skills", label: "Skills", icon: Boxes },
      { href: "/rooms", label: "Rooms", icon: Radio },
      { href: "/runs", label: "Runs", icon: History },
    ],
  },
  {
    label: "Account",
    items: [
      { href: "/keys", label: "API Keys", icon: KeyRound },
      { href: "/providers", label: "Providers", icon: Plug },
      { href: "/usage", label: "Usage", icon: Gauge },
      { href: "/connections", label: "Connections", icon: Plug },
      { href: "/settings", label: "Settings", icon: SettingsIcon },
    ],
  },
];

function RailLink({ item, active }: { item: NavItem; active: boolean }) {
  const Icon = item.icon;
  return (
    <Link
      href={item.href}
      aria-current={active ? "page" : undefined}
      className={cn(
        "relative flex items-center gap-2.5 rounded-md px-2.5 py-1.5 font-mono text-label transition-colors",
        active
          ? "bg-[var(--rail-accent-tint)] text-[var(--rail-ink)] before:absolute before:inset-y-1 before:left-0 before:w-0.5 before:rounded-full before:bg-[var(--rail-accent)] before:content-['']"
          : "text-[var(--rail-muted)] hover:bg-[var(--rail-bg-2)] hover:text-[var(--rail-ink)]",
      )}
    >
      <Icon size={15} className={active ? "text-[var(--rail-accent)]" : ""} />
      {item.label}
    </Link>
  );
}

export function Rail({ horizontal = false }: { horizontal?: boolean }) {
  const pathname = usePathname() ?? "";
  const isActive = (href: string) => pathname === href || pathname.startsWith(`${href}/`);

  if (horizontal) {
    // Below 820px the rail collapses to a horizontal scrollable strip.
    const all = [...TOP, ...GROUPS.flatMap((g) => g.items)];
    return (
      <nav className="rail-shell flex w-full items-center gap-1 overflow-x-auto border-b border-[var(--rail-line)] bg-[var(--rail-bg)] px-2 py-2">
        {all.map((item) => (
          <div key={item.href} className="shrink-0">
            <RailLink item={item} active={isActive(item.href)} />
          </div>
        ))}
      </nav>
    );
  }

  return (
    <aside
      className="rail-shell z-20 hidden h-full shrink-0 flex-col border-r border-[var(--rail-line)] bg-[var(--rail-bg)] md:flex"
      style={{ width: "var(--rail-w)" }}
    >
      <div className="flex items-center gap-2 px-3 py-3">
        <div className="grid h-7 w-7 place-items-center rounded-md bg-ox font-mono text-[13px] font-bold text-white">
          H
        </div>
        <span className="font-title text-subhead text-[var(--rail-ink)]">Harness</span>
      </div>

      {/* tenant selector */}
      <button className="mx-3 mb-2 flex items-center justify-between rounded-md border border-[var(--rail-line)] bg-[var(--rail-bg-2)] px-2.5 py-1.5 font-mono text-label text-[var(--rail-ink)] hover:bg-[var(--rail-bg-3)]">
        <span className="truncate">{process.env.NEXT_PUBLIC_DEFAULT_TENANT ?? "default"}</span>
        <ChevronDown size={13} className="text-[var(--rail-muted)]" />
      </button>

      <nav className="flex flex-1 flex-col gap-0.5 overflow-y-auto px-3 pb-2">
        {TOP.map((item) => (
          <RailLink key={item.href} item={item} active={isActive(item.href)} />
        ))}
        {GROUPS.map((group) => (
          <div key={group.label} className="mt-4 flex flex-col gap-0.5">
            <div className="rail-group-label px-2.5 pb-1">{group.label}</div>
            {group.items.map((item) => (
              <RailLink key={item.href} item={item} active={isActive(item.href)} />
            ))}
          </div>
        ))}
      </nav>

      <UsagePulse />
    </aside>
  );
}
