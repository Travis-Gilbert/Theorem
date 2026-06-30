import Link from "next/link";
import { Sidebar } from "@/components/marketing/Sidebar";
import { Hero } from "@/components/marketing/Hero";
import { Programmable } from "@/components/marketing/Programmable";
import { Footer } from "@/components/marketing/Footer";

export default function WelcomePage() {
  return (
    <div id="main-content">
      <div className="flex">
        {/* persistent left rail — the harness workspace, on the landing */}
        <Sidebar />

        <main className="min-w-0 flex-1">
          {/* mobile bar — the doc tree is desktop-only, so small screens get the
              wordmark + console here instead of the sidebar rail */}
          <header className="flex items-center justify-between px-5 py-4 lg:hidden">
            <b className="font-title text-[18px]" style={{ fontWeight: 500 }}>
              Theorem&apos;s Harness
            </b>
            <Link
              href="/canvas"
              className="rounded-[10px] px-3.5 py-2 text-[13px] font-semibold text-ink shadow-elev-1"
              style={{ background: "var(--raise)" }}
            >
              Open console
            </Link>
          </header>

          {/* nav (brand lives in the sidebar now) — desktop only */}
          <nav className="sticky top-0 z-40 hidden items-center justify-end gap-1.5 px-8 py-6 text-[14px] lg:flex">
            <NavLink href="#how">How it works</NavLink>
            <NavLink href="#graph">The graph</NavLink>
            <NavLink href="#build">Build</NavLink>
            <NavLink href="/canvas">Sign in</NavLink>
            <Link
              href="/canvas"
              className="ml-1 rounded-[10px] px-4 py-2.5 font-semibold text-ink shadow-elev-1 transition-colors"
              style={{ background: "var(--raise)" }}
            >
              Open console
            </Link>
          </nav>

          <Hero />

          <Programmable />
        </main>
      </div>

      <Footer />
    </div>
  );
}

function NavLink({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <Link
      href={href}
      className="rounded-md px-3.5 py-2 font-medium text-muted-foreground transition-colors hover:bg-black/5 hover:text-ink"
    >
      {children}
    </Link>
  );
}
