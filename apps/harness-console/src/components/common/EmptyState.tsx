import * as React from "react";

/** Warmth lives in empty states. A clear prompt plus one primary action, never
 *  a dead canvas. */
export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
}: {
  icon?: React.ComponentType<{ size?: number; className?: string }>;
  title: string;
  description?: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="material flex flex-col items-center justify-center gap-3 px-6 py-12 text-center">
      {Icon && (
        <div className="grid h-11 w-11 place-items-center rounded-full bg-surface-2 text-muted-foreground">
          <Icon size={20} />
        </div>
      )}
      <div>
        <p className="font-title text-subhead text-ink">{title}</p>
        {description && <p className="mt-1 max-w-md text-label text-muted-foreground">{description}</p>}
      </div>
      {action}
    </div>
  );
}
