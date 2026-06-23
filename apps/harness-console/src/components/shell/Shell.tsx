"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { Rail } from "./Rail";
import { TopBar } from "./TopBar";
import { DynamicIsland } from "@/components/island/DynamicIsland";
import { useMediaQuery } from "@/lib/hooks/useMediaQuery";

// Canvas and Inbox are full-bleed instruments: they own the whole content area
// and manage their own scrolling, rather than sitting in the centered reading
// column the document surfaces use.
const FULL_BLEED = ["/canvas", "/inbox"];

export function Shell({ children }: { children: React.ReactNode }) {
  const narrow = useMediaQuery("(max-width: 820px)");
  const pathname = usePathname() ?? "";
  const fullBleed = FULL_BLEED.some((p) => pathname === p || pathname.startsWith(`${p}/`));

  return (
    <div className="flex h-dvh w-full overflow-hidden">
      {!narrow && <Rail />}
      <div className="flex min-w-0 flex-1 flex-col">
        {narrow && <Rail horizontal />}
        <TopBar />
        <main id="main-content" tabIndex={-1} className="relative flex min-h-0 flex-1 flex-col overflow-hidden focus:outline-none">
          {fullBleed ? (
            <div className="min-h-0 flex-1">{children}</div>
          ) : (
            // The scroll container the Dynamic Island scroll-spy watches.
            <div id="content-well" data-content-well className="flex-1 overflow-y-auto">
              <div className="mx-auto w-full max-w-6xl px-5 py-6 pb-32">{children}</div>
            </div>
          )}
        </main>
      </div>
      <DynamicIsland />
    </div>
  );
}
