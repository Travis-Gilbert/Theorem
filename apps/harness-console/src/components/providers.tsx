"use client";

import * as React from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Toaster } from "@/components/ui/toaster";
import { ConsoleProvider } from "@/components/island/console-context";

/** Client provider stack: tooltips, console UI state, toasts. The theme is set
 *  on <html data-theme> by Settings; default is light. */
export function Providers({ children }: { children: React.ReactNode }) {
  return (
    <TooltipProvider delayDuration={200}>
      <ConsoleProvider>
        {children}
        <Toaster />
      </ConsoleProvider>
    </TooltipProvider>
  );
}
