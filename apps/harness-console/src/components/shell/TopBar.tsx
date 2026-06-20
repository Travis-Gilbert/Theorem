"use client";

import { Search, CircleHelp, Command as CommandIcon } from "lucide-react";
import { useConsole } from "@/components/island/console-context";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

/**
 * Top bar: the omnibar (centered, persistent) plus an account avatar and a help
 * control. The omnibar is the universal entry point; clicking it or Cmd/Ctrl K
 * opens the command palette (the Dynamic Island command state). It resolves
 * typed input as > verbs, @ nav, # tags, or plain-text graph search with an
 * "Ask the Theorem agent" fallthrough.
 */
export function TopBar() {
  const { setPaletteOpen } = useConsole();

  return (
    <header
      className="z-30 flex shrink-0 items-center gap-3 border-b border-line bg-bg px-4"
      style={{ height: "var(--topbar-h)" }}
    >
      <button
        onClick={() => setPaletteOpen(true)}
        className="group mx-auto flex h-9 w-full max-w-xl items-center gap-2 rounded-lg border border-line bg-surface px-3 text-muted-foreground transition-colors hover:bg-surface-2"
        aria-label="Open omnibar"
      >
        <Search size={15} />
        <span className="flex-1 text-left font-mono text-label">Search, run a verb, or ask the Theorem agent</span>
        <span className="flex items-center gap-1 rounded border border-line px-1.5 py-0.5 font-mono text-[10px] text-faint">
          <CommandIcon size={10} />K
        </span>
      </button>

      <button className="rounded-md p-2 text-muted-foreground hover:bg-surface-2 hover:text-ink" aria-label="Help">
        <CircleHelp size={16} />
      </button>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            className="grid h-8 w-8 place-items-center rounded-full bg-ox font-mono text-label font-bold text-white"
            aria-label="Account"
          >
            T
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem>Account</DropdownMenuItem>
          <DropdownMenuItem>Tenants</DropdownMenuItem>
          <DropdownMenuItem>Theme</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem>Sign out</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </header>
  );
}
