"use client";

import { useEffect, useRef } from "react";
import { createDraggable, animate, utils, createSpring } from "animejs";
import { Hand } from "lucide-react";

/** The hero: an implicit trail of memory cards that you drag-shuffle through. The
 *  front is the parchment headline; behind it, abstract modality-wash cards
 *  recede into the distance in 3D (the "trail of memories" — no labels). Flick
 *  the front card and it tucks to the back; the next card comes forward.
 *
 *  Merges the two earlier ideas: the StackedPanels slant/trail (perspective +
 *  preserve-3d wrapper + translateZ depth) with the anime.js drag-shuffle. The
 *  draggable owns ONE card (the front); on release it revert()s so the tween can
 *  take the transform cleanly, then a fresh draggable arms the new front. Washes
 *  are pure CSS (no network). Mobile renders a plain headline (Hero.tsx). */

const W = 520;
const H = 332;
const THROW = 110; // px from center, or a hard flick, to count as "shuffled"
const SPRING = createSpring({ stiffness: 120, damping: 16 });

// the slant (reduced from the old -34) — gentler, but clearly a 3D trail
const TILT_Y = -22;
const TILT_X = 9;

// resting transform by depth (0 = front): recede in Z + peek down + shrink + fan
const restX = (d: number) => d * 8;
const restY = (d: number) => d * 26;
const restZ = (d: number) => -d * 48;
const restRot = (d: number) => d * 1.5;
const restScale = (d: number) => 1 - d * 0.04;
const restOpacity = (d: number) => (d === 0 ? 1 : Math.max(0.72, 1 - d * 0.09));

export function MemoryDeck() {
  const cardRefs = useRef<(HTMLElement | null)[]>([]);
  const order = useRef<number[]>(CARDS.map((_, i) => i));
  const drag = useRef<ReturnType<typeof createDraggable> | null>(null);
  const reduce = useRef(false);

  useEffect(() => {
    reduce.current = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    const layout = (animated: boolean) => {
      order.current.forEach((cardIdx, depth) => {
        const el = cardRefs.current[cardIdx];
        if (!el) return;
        el.style.zIndex = String(100 - depth);
        const props = {
          x: restX(depth),
          y: restY(depth),
          z: restZ(depth),
          rotate: restRot(depth),
          scale: restScale(depth),
          opacity: restOpacity(depth),
        };
        if (animated && !reduce.current) animate(el, { ...props, duration: 520, ease: SPRING });
        else utils.set(el, props);
      });
    };

    const mountFront = () => {
      const el = cardRefs.current[order.current[0]];
      if (!el) return;
      drag.current = createDraggable(el, {
        onResize: () => layout(false),
        onRelease: (self) => {
          const x = self.x;
          const y = self.y;
          const flung = Math.hypot(x, y) > THROW || self.velocity > 1.4;
          self.revert(); // destroy this draggable; releases its hold on the transform
          utils.set(el, { x, y }); // re-pin where it was let go (no jump)
          if (flung) {
            order.current = [...order.current.slice(1), order.current[0]]; // front -> back
            layout(true);
            mountFront(); // arm the new front
          } else if (reduce.current) {
            utils.set(el, { x: 0, y: 0, rotate: 0, scale: 1 });
            mountFront();
          } else {
            animate(el, { x: 0, y: 0, rotate: 0, scale: 1, duration: 460, ease: SPRING, onComplete: mountFront });
          }
        },
      });
    };

    layout(false);
    mountFront();
    const onResize = () => layout(false);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      drag.current?.revert();
    };
  }, []);

  return (
    <div className="relative flex h-full w-full select-none items-center justify-center" style={{ perspective: "1400px" }}>
      <div
        style={{
          position: "relative",
          width: 0,
          height: 0,
          transformStyle: "preserve-3d",
          transform: `rotateX(${TILT_X}deg) rotateY(${TILT_Y}deg)`,
        }}
      >
        {CARDS.map((c, i) => (
          <article
            key={c.id}
            ref={(el) => {
              cardRefs.current[i] = el;
            }}
            className="absolute left-1/2 top-1/2 cursor-grab overflow-hidden rounded-2xl border active:cursor-grabbing"
            style={{
              width: W,
              height: H,
              marginLeft: -W / 2,
              marginTop: -H / 2,
              borderColor: c.wash ? "rgba(255,255,255,.14)" : "var(--line)",
              boxShadow: "0 26px 60px rgba(20,16,14,.42)",
              willChange: "transform",
            }}
          >
            {c.wash ? <WashFace wash={c.wash} /> : <HeadlineFace />}
          </article>
        ))}
      </div>
    </div>
  );
}

/** Front card: the parchment headline. */
function HeadlineFace() {
  return (
    <div
      className="flex h-full flex-col justify-center p-9"
      style={{ background: "linear-gradient(180deg, var(--raise), var(--surface))" }}
    >
      <span className="mk-eyebrow mb-4">A coordination layer for AI agents</span>
      <h1 className="text-[clamp(40px,3.4vw,56px)] leading-[0.96]">
        Many models,
        <br />
        one agent.
      </h1>
      <div className="mt-6 flex flex-wrap gap-4 text-[13px] text-muted-foreground">
        <Pill color="var(--green)">No credit card</Pill>
        <Pill color="var(--teal)">Self-hostable</Pill>
        <Pill color="var(--gold)">Local-first memory</Pill>
      </div>
      <span className="mk-eyebrow pointer-events-none absolute bottom-5 left-9 inline-flex items-center gap-1.5 opacity-60">
        <Hand size={12} /> drag to shuffle
      </span>
    </div>
  );
}

/** Implicit memory card: an abstract modality wash over a dark base. No label. */
function WashFace({ wash }: { wash: string }) {
  return (
    <div className="relative h-full w-full" style={{ background: "var(--ink)" }}>
      <div className="absolute inset-0" style={{ background: wash, mixBlendMode: "screen" }} />
      <div className="absolute inset-0" style={{ background: "linear-gradient(to bottom, rgba(0,0,0,.04), rgba(0,0,0,.34))" }} />
      <div className="absolute inset-0 rounded-2xl" style={{ boxShadow: "inset 0 1px 0 rgba(255,255,255,.12)" }} />
    </div>
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

// front = headline; the rest = implicit modality washes (image · code · web · record)
const CARDS: { id: string; wash?: string }[] = [
  { id: "headline" },
  { id: "m1", wash: "linear-gradient(135deg, rgba(74,122,154,.62), rgba(45,95,107,.5))" },
  { id: "m2", wash: "linear-gradient(135deg, rgba(184,98,61,.6), rgba(196,154,74,.42))" },
  { id: "m3", wash: "linear-gradient(135deg, rgba(90,122,74,.6), rgba(45,95,107,.42))" },
  { id: "m4", wash: "linear-gradient(135deg, rgba(196,154,74,.58), rgba(184,98,61,.42))" },
];
