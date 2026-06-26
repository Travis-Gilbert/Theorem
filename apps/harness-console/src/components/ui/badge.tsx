import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center gap-1 rounded border px-1.5 py-0.5 font-mono text-[11px] leading-none",
  {
    variants: {
      tone: {
        neutral: "border-line bg-surface-2 text-muted-foreground",
        accent: "border-ox bg-ox-tint text-ox",
        live: "border-live text-live",
        warn: "border-warn text-warn",
        ink: "border-line bg-ink text-bg",
      },
    },
    defaultVariants: { tone: "neutral" },
  },
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLSpanElement>,
    VariantProps<typeof badgeVariants> {}

export function Badge({ className, tone, ...props }: BadgeProps) {
  return <span className={cn(badgeVariants({ tone }), className)} {...props} />;
}
