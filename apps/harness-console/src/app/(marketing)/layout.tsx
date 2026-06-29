import type { Metadata } from "next";
import localFont from "next/font/local";
import { IBM_Plex_Sans } from "next/font/google";
import "./marketing.css";

// Amarna (semi-glyphic humanist sans) — the marketing display face, self-hosted.
const amarna = localFont({
  src: "./Amarna.woff2",
  variable: "--font-amarna",
  display: "swap",
});

// Regular IBM Plex Sans for marketing body (the console uses the Condensed cut).
const plexSans = IBM_Plex_Sans({
  subsets: ["latin"],
  weight: ["400", "500", "600", "700"],
  variable: "--font-plex-sans-reg",
  display: "swap",
});

export const metadata: Metadata = {
  title: "Theorem's Harness — shared memory and coordination for your agents",
  description:
    "A coordination layer for AI agents: one durable, graph-native memory and a room to work together without stepping on each other.",
};

export default function MarketingLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className={`${amarna.variable} ${plexSans.variable} marketing`}>{children}</div>
  );
}
