"use client";

import { cn } from "@/lib/utils";
import { Sparkles } from "lucide-react";

export interface DisplayCardProps {
  className?: string;
  icon?: React.ReactNode;
  title?: string;
  description?: string;
  date?: string;
  titleClassName?: string;
  badgeClassName?: string;
}

/** Stacked, skewed reveal cards (from the supplied DisplayCards), retokenized to
 *  the marketing surface: parchment cards, brand badges, grayscale-until-hover. */
export function DisplayCard({
  className,
  icon = <Sparkles className="size-4" />,
  title = "Featured",
  description = "Discover amazing content",
  date = "Just now",
  titleClassName = "text-ink",
  badgeClassName = "bg-foreground/10",
}: DisplayCardProps) {
  return (
    <div
      className={cn(
        "relative flex h-36 w-[22rem] -skew-y-[8deg] select-none flex-col justify-between rounded-xl border-2 border-line bg-muted/70 px-4 py-3 backdrop-blur-sm transition-all duration-700 after:absolute after:-right-1 after:top-[-5%] after:h-[110%] after:w-[20rem] after:bg-gradient-to-l after:from-background after:to-transparent after:content-[''] hover:border-line-strong hover:bg-muted [&>*]:flex [&>*]:items-center [&>*]:gap-2",
        className,
      )}
    >
      <div>
        <span className={cn("relative inline-block rounded-full p-1.5", badgeClassName)}>{icon}</span>
        <p className={cn("text-lg font-medium", titleClassName)}>{title}</p>
      </div>
      <p className="whitespace-nowrap text-lg text-ink">{description}</p>
      <p className="text-muted-foreground">{date}</p>
    </div>
  );
}
