"use client";

import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

// Axis 4: one primary action per region. `primary` is the oxblood action; use it
// once per surface. Everything else is neutral structure.
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-label font-mono font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--ox-ring)] disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        primary: "bg-ox text-white hover:bg-ox-hover",
        outline: "border border-line bg-bg text-ink hover:bg-surface-2",
        ghost: "text-ink hover:bg-surface-2",
        subtle: "bg-surface-2 text-ink hover:bg-surface",
        danger: "border border-line text-ox hover:bg-ox-tint",
        link: "text-ox underline-offset-4 hover:underline",
      },
      size: {
        sm: "h-8 px-3",
        md: "h-9 px-4",
        lg: "h-10 px-5",
        icon: "h-9 w-9",
      },
    },
    defaultVariants: { variant: "outline", size: "md" },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return <Comp ref={ref} className={cn(buttonVariants({ variant, size }), className)} {...props} />;
  },
);
Button.displayName = "Button";

export { buttonVariants };
