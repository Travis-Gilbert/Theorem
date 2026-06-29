import { MemoryDeck } from "@/components/marketing/MemoryDeck";

/** Hero. Desktop (lg+): an implicit trail of memory cards you drag-shuffle
 *  through — the headline is the front card, abstract modality washes recede
 *  behind it in 3D. Mobile: the fixed-size 3D deck can't fit a phone, so we
 *  render a plain headline block instead. */
export function Hero() {
  return (
    <section className="relative overflow-hidden">
      {/* mobile: plain headline (no fixed-size 3D cards to clip) */}
      <div className="px-6 pb-14 pt-6 lg:hidden">
        <span className="mk-eyebrow mb-4">A coordination layer for AI agents</span>
        <h1 className="text-[clamp(40px,3.4vw,58px)] leading-[0.96]">
          Many models,
          <br />
          one agent.
        </h1>
        <div className="mt-6 flex flex-wrap gap-4 text-[13px] text-muted-foreground">
          <Pill color="var(--green)">No credit card</Pill>
          <Pill color="var(--teal)">Self-hostable</Pill>
          <Pill color="var(--gold)">Local-first memory</Pill>
        </div>
      </div>

      {/* desktop: the drag-shuffle memory trail */}
      <div className="hidden min-h-[680px] lg:block">
        <div className="absolute inset-0">
          <MemoryDeck />
        </div>
      </div>
    </section>
  );
}

function Pill({ color, children }: { color: string; children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-2">
      <i className="h-1.5 w-1.5 rounded-full" style={{ background: color }} />
      {children}
    </span>
  );
}
