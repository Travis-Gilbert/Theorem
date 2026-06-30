"use client";

import React from "react";
import type { ComponentProps, ReactNode, CSSProperties } from "react";
import { motion, useReducedMotion } from "motion/react";
import { Github, Twitter, Linkedin, type LucideIcon } from "lucide-react";

/** Dark charcoal footer (adapted from the supplied footer-section). Sits as a
 *  rounded-top dark panel at the bottom of the parchment page — the "engine"
 *  ground. Scopes light token overrides so the shared text-ink/text-muted
 *  utilities resolve light inside it. */

type FooterLink = { title: string; href: string; icon?: LucideIcon };
type FooterSection = { label: string; links: FooterLink[] };

const GITBOOK = "https://travis-gilbert.gitbook.io/theorems-harness";

const SECTIONS: FooterSection[] = [
  {
    label: "Concepts",
    links: [
      { title: "What is Theorem", href: `${GITBOOK}/concepts/what-is-theorem` },
      { title: "The Harness", href: `${GITBOOK}/concepts/the-harness` },
      { title: "The graph store", href: `${GITBOOK}/concepts/substrate-graphstore` },
      { title: "Mental model", href: `${GITBOOK}/mental-model` },
    ],
  },
  {
    label: "Build",
    links: [
      { title: "Getting started", href: `${GITBOOK}/getting-started` },
      { title: "HTTP API", href: `${GITBOOK}/reference/api-http` },
      { title: "MCP tools", href: `${GITBOOK}/reference/mcp-tools` },
      { title: "SDKs", href: `${GITBOOK}/reference/sdks` },
    ],
  },
  {
    label: "Reference",
    links: [
      { title: "Glossary", href: `${GITBOOK}/reference/glossary` },
      { title: "Crates", href: `${GITBOOK}/reference/crates` },
      { title: "Apps", href: `${GITBOOK}/reference/apps` },
      { title: "Architecture", href: `${GITBOOK}/architecture/overview` },
    ],
  },
  {
    label: "Connect",
    links: [
      { title: "GitHub", href: "https://github.com/Travis-Gilbert", icon: Github },
      { title: "X", href: "#", icon: Twitter },
      { title: "LinkedIn", href: "#", icon: Linkedin },
      { title: "Open console", href: "/canvas" },
    ],
  },
];

const darkScope: CSSProperties = {
  // charcoal ground (a touch lighter than the near-black shell) + light tokens
  background:
    "radial-gradient(35% 128px at 50% 0%, rgba(255,255,255,.06), transparent), #242427",
  ["--ink" as string]: "#ededf0",
  ["--muted" as string]: "#a4a09a",
  ["--line" as string]: "rgba(255,255,255,.1)",
};

export function Footer() {
  return (
    <footer
      className="relative mx-auto mt-20 flex w-full max-w-[1180px] flex-col items-center justify-center rounded-t-[32px] border-t border-line px-6 py-14 lg:py-16"
      style={darkScope}
    >
      <div className="absolute left-1/2 right-1/2 top-0 h-px w-1/3 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white/20 blur" />

      <div className="grid w-full gap-10 xl:grid-cols-3 xl:gap-8">
        <AnimatedContainer className="space-y-4">
          <div className="font-title text-[22px] text-ink" style={{ fontWeight: 500 }}>
            Theorem&apos;s Harness
          </div>
          <p className="max-w-[34ch] text-sm text-muted-foreground">
            One durable, graph-native memory and a room where your agents coordinate. Self-hostable,
            local-first, open from the Rust core up.
          </p>
          <p className="pt-2 text-xs text-muted-foreground">
            © {new Date().getFullYear()} Theorem · the Rust-native graph and harness
          </p>
        </AnimatedContainer>

        <div className="mt-2 grid grid-cols-2 gap-8 md:grid-cols-4 xl:col-span-2 xl:mt-0">
          {SECTIONS.map((section, index) => (
            <AnimatedContainer key={section.label} delay={0.1 + index * 0.08}>
              <div>
                <h3 className="font-mono text-[13px] uppercase tracking-[0.1em] text-muted-foreground">
                  {section.label}
                </h3>
                <ul className="mt-4 space-y-2.5 text-[15px] text-muted-foreground">
                  {section.links.map((link) => (
                    <li key={link.title}>
                      <a
                        href={link.href}
                        className="inline-flex items-center transition-colors duration-200 hover:text-ink"
                      >
                        {link.icon && <link.icon className="me-1.5 size-4" />}
                        {link.title}
                      </a>
                    </li>
                  ))}
                </ul>
              </div>
            </AnimatedContainer>
          ))}
        </div>
      </div>
    </footer>
  );
}

type ViewAnimationProps = {
  delay?: number;
  className?: ComponentProps<typeof motion.div>["className"];
  children: ReactNode;
};

function AnimatedContainer({ className, delay = 0.1, children }: ViewAnimationProps) {
  const shouldReduceMotion = useReducedMotion();
  if (shouldReduceMotion) {
    return <div className={className}>{children}</div>;
  }
  return (
    <motion.div
      initial={{ filter: "blur(4px)", translateY: -8, opacity: 0 }}
      whileInView={{ filter: "blur(0px)", translateY: 0, opacity: 1 }}
      viewport={{ once: true }}
      transition={{ delay, duration: 0.8 }}
      className={className}
    >
      {children}
    </motion.div>
  );
}
