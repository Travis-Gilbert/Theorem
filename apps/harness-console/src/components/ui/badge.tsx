import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center gap-1 rounded border px-1.5 py-0.5 font-mono text-[11px] leading-none",
  {
    variants: {
      tone: {
        neutral: "border-line bg-surface-2 text-muted-foreground",
        accent: "border-[var(--ox)] bg-[var(--ox-tint)] text-ox",
        live: "border-[var(--live)] text-[var(--live)]",
        warn: "border-[var(--warn)] text-[var(--warn)]",
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
