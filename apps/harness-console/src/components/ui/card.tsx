import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * Card = the materiality surface (depth Layer 2). Opaque grey, faint grain,
 * hairline, elevation token. `lift` adds the hover document-off-a-stack feel.
 * `blueprint` overlays the 40px grid. Dense content (tables, ledgers) should
 * pass `calm` to drop grain and stay flat for legibility.
 */
export function Card({
  className,
  lift,
  blueprint,
  calm,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & { lift?: boolean; blueprint?: boolean; calm?: boolean }) {
  return (
    <div
      className={cn(
        calm ? "rounded-lg border border-line bg-surface" : "material",
        lift && "material-lift",
        blueprint && "material-blueprint",
        className,
      )}
      {...props}
    />
  );
}

export function CardHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("flex flex-col gap-1 p-4", className)} {...props} />;
}

export function CardTitle({ className, ...props }: React.HTMLAttributes<HTMLHeadingElement>) {
  return <h3 className={cn("font-title text-subhead text-ink", className)} {...props} />;
}

export function CardDescription({ className, ...props }: React.HTMLAttributes<HTMLParagraphElement>) {
  return <p className={cn("text-label text-muted-foreground", className)} {...props} />;
}

export function CardContent({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("p-4 pt-0", className)} {...props} />;
}

export function CardFooter({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("flex items-center gap-2 p-4 pt-0", className)} {...props} />;
}
