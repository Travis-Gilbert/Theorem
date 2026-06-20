"use client";

import * as React from "react";
import * as SwitchPrimitive from "@radix-ui/react-switch";
import * as LabelPrimitive from "@radix-ui/react-label";
import * as ScrollAreaPrimitive from "@radix-ui/react-scroll-area";
import * as SeparatorPrimitive from "@radix-ui/react-separator";
import { cn } from "@/lib/utils";

// Switch -------------------------------------------------------------------
export const Switch = React.forwardRef<
  React.ElementRef<typeof SwitchPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitive.Root>
>(({ className, ...props }, ref) => (
  <SwitchPrimitive.Root
    ref={ref}
    className={cn(
      "peer inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border border-line transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--ox-ring)] data-[state=checked]:bg-ox data-[state=unchecked]:bg-surface-2",
      className,
    )}
    {...props}
  >
    <SwitchPrimitive.Thumb className="pointer-events-none block h-4 w-4 translate-x-0.5 rounded-full bg-bg shadow transition-transform data-[state=checked]:translate-x-4" />
  </SwitchPrimitive.Root>
));
Switch.displayName = "Switch";

// Label --------------------------------------------------------------------
export const Label = React.forwardRef<
  React.ElementRef<typeof LabelPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof LabelPrimitive.Root>
>(({ className, ...props }, ref) => (
  <LabelPrimitive.Root
    ref={ref}
    className={cn("font-mono text-label text-muted-foreground", className)}
    {...props}
  />
));
Label.displayName = "Label";

// Separator ----------------------------------------------------------------
export function Separator({ className, orientation = "horizontal", ...props }: React.ComponentPropsWithoutRef<typeof SeparatorPrimitive.Root>) {
  return (
    <SeparatorPrimitive.Root
      orientation={orientation}
      className={cn(orientation === "horizontal" ? "h-px w-full" : "h-full w-px", "bg-line", className)}
      {...props}
    />
  );
}

// ScrollArea ---------------------------------------------------------------
export function ScrollArea({ className, children, ...props }: React.ComponentPropsWithoutRef<typeof ScrollAreaPrimitive.Root>) {
  return (
    <ScrollAreaPrimitive.Root className={cn("relative overflow-hidden", className)} {...props}>
      <ScrollAreaPrimitive.Viewport className="h-full w-full rounded-[inherit]">{children}</ScrollAreaPrimitive.Viewport>
      <ScrollAreaPrimitive.Scrollbar orientation="vertical" className="flex w-2 touch-none select-none p-0.5">
        <ScrollAreaPrimitive.Thumb className="flex-1 rounded-full bg-faint" />
      </ScrollAreaPrimitive.Scrollbar>
    </ScrollAreaPrimitive.Root>
  );
}

// Skeleton -----------------------------------------------------------------
export function Skeleton({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("animate-[pulse_1.5s_ease-in-out_infinite] rounded bg-surface-2", className)} {...props} />;
}
