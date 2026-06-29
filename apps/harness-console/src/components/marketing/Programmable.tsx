import { DisplayCard } from "@/components/ui/display-cards";

/** "Programmable & extendable" — the pivotal, not-yet-in-docs story: the engine
 *  speaks Postgres and Redis on the wire, and agents extend it with declarative
 *  skills or sandboxed WASM plugins (like a database extension). Card glyphs are
 *  the real brand logos from the a-file-icon pack (/public/file-icons). */

// brand logo sitting directly on the card (no badge) — the mark's own color is the accent
function Brand({ src }: { src: string }) {
  // eslint-disable-next-line @next/next/no-img-element
  return <img src={src} alt="" className="size-6" />;
}
const BRAND_BADGE = "bg-transparent p-0 ring-0";

const STACK = [
  "[grid-area:stack] hover:-translate-y-10 before:absolute before:left-0 before:top-0 before:h-[100%] before:w-[100%] before:rounded-xl before:bg-background/50 before:bg-blend-overlay before:outline-1 before:outline-border before:content-[''] before:transition-opacity before:duration-700 grayscale-[100%] hover:grayscale-0 hover:before:opacity-0",
  "[grid-area:stack] translate-x-16 translate-y-10 hover:-translate-y-1 before:absolute before:left-0 before:top-0 before:h-[100%] before:w-[100%] before:rounded-xl before:bg-background/50 before:bg-blend-overlay before:outline-1 before:outline-border before:content-[''] before:transition-opacity before:duration-700 grayscale-[100%] hover:grayscale-0 hover:before:opacity-0",
  "[grid-area:stack] translate-x-32 translate-y-20 hover:translate-y-10",
];

export function Programmable() {
  return (
    <section id="how" className="mx-auto max-w-[1180px] px-8 pb-28">
      <div className="max-w-[660px]">
        <span className="mk-eyebrow mb-3.5">Programmable &amp; extensible</span>
        <h2 className="text-[clamp(30px,3.8vw,46px)] leading-[1.04]">Programmable and extendable.</h2>
        <p className="mt-4 max-w-[58ch] text-[17px] leading-relaxed text-muted-foreground">
          Talk to it over the Postgres and Redis wire protocols — and let agents extend the engine
          itself with declarative skills or sandboxed WASM plugins, the way an extension adds functions
          to a database.
        </p>
      </div>

      <div className="mt-14 grid min-h-[300px] place-items-center [grid-template-areas:'stack']">
        <DisplayCard
          className={STACK[0]}
          icon={<Brand src="/file-icons/postgresql.svg" />}
          badgeClassName={BRAND_BADGE}
          title="Postgres wire"
          description="Query it with psql, or any client"
          date="wire protocol"
        />
        <DisplayCard
          className={STACK[1]}
          icon={<Brand src="/file-icons/redis.svg" />}
          badgeClassName={BRAND_BADGE}
          title="Redis (RESP)"
          description="A native RESP command loop"
          date="wire protocol"
        />
        <DisplayCard
          className={STACK[2]}
          icon={<Brand src="/file-icons/webassembly.svg" />}
          badgeClassName={BRAND_BADGE}
          title="WASM plugins"
          description="Agents extend it, sandboxed"
          date="like a DB extension"
        />
      </div>
    </section>
  );
}
