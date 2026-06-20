"use client";

import * as React from "react";
import { Command as CommandPrimitive } from "cmdk";
import { Search } from "lucide-react";
import { cn } from "@/lib/utils";

/** cmdk-based command primitives, retokenized. Used by the Dynamic Island
 *  command-palette state and the omnibar. */
export const Command = React.forwardRef<
  React.ElementRef<typeof CommandPrimitive>,
  React.ComponentPropsWithoutRef<typeof CommandPrimitive>
>(({ className, ...props }, ref) => (
  <CommandPrimitive
    ref={ref}
    className={cn("flex h-full w-full flex-col overflow-hidden rounded-md bg-bg text-ink", className)}
    {...props}
  />
));
Command.displayName = "Command";

export function CommandInput({ className, ...props }: React.ComponentPropsWithoutRef<typeof CommandPrimitive.Input>) {
  return (
    <div className="flex items-center gap-2 border-b border-line px-3">
      <Search size={15} className="text-muted-foreground" />
      <CommandPrimitive.Input
        className={cn(
          "h-11 w-full bg-transparent font-mono text-body text-ink outline-none placeholder:text-faint",
          className,
        )}
        {...props}
      />
    </div>
  );
}

export const CommandList = React.forwardRef<
  React.ElementRef<typeof CommandPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof CommandPrimitive.List>
>(({ className, ...props }, ref) => (
  <CommandPrimitive.List ref={ref} className={cn("max-h-80 overflow-y-auto p-1", className)} {...props} />
));
CommandList.displayName = "CommandList";

export function CommandEmpty(props: React.ComponentPropsWithoutRef<typeof CommandPrimitive.Empty>) {
  return <CommandPrimitive.Empty className="px-3 py-6 text-center text-label text-muted-foreground" {...props} />;
}

export function CommandGroup({ className, ...props }: React.ComponentPropsWithoutRef<typeof CommandPrimitive.Group>) {
  return (
    <CommandPrimitive.Group
      className={cn("[&_[cmdk-group-heading]]:rail-group-label [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5", className)}
      {...props}
    />
  );
}

export const CommandItem = React.forwardRef<
  React.ElementRef<typeof CommandPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof CommandPrimitive.Item>
>(({ className, ...props }, ref) => (
  <CommandPrimitive.Item
    ref={ref}
    className={cn(
      "flex cursor-pointer select-none items-center gap-2 rounded px-2 py-2 text-body text-ink outline-none data-[selected=true]:bg-surface-2",
      className,
    )}
    {...props}
  />
));
CommandItem.displayName = "CommandItem";
