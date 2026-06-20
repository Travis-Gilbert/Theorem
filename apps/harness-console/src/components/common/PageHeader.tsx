import * as React from "react";
import { cn } from "@/lib/utils";

/** Surface header: serif title (Vollkorn), mono eyebrow, optional actions. The
 *  title carries data-toc so it registers as the first TOC entry. */
export function PageHeader({
  title,
  eyebrow,
  description,
  actions,
  id,
}: {
  title: string;
  eyebrow?: string;
  description?: string;
  actions?: React.ReactNode;
  id?: string;
}) {
  const anchor = id ?? title.toLowerCase().replace(/\s+/g, "-");
  return (
    <div className="mb-6 flex items-start justify-between gap-4">
      <div className="min-w-0">
        {eyebrow && <div className="rail-group-label mb-1">{eyebrow}</div>}
        <h1 id={anchor} data-toc data-toc-title={title} data-toc-depth={1} className="font-title text-title text-ink">
          {title}
        </h1>
        {description && <p className="mt-1 max-w-[var(--measure)] text-body text-muted-foreground">{description}</p>}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </div>
  );
}

/** A content section that registers with the Dynamic Island TOC. */
export function Section({
  id,
  title,
  depth = 2,
  className,
  children,
  actions,
}: {
  id: string;
  title: string;
  depth?: number;
  className?: string;
  children: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <section className={cn("mb-8", className)}>
      <div className="mb-3 flex items-center justify-between">
        <h2 id={id} data-toc data-toc-title={title} data-toc-depth={depth} className="font-title text-subhead text-ink">
          {title}
        </h2>
        {actions}
      </div>
      {children}
    </section>
  );
}
