import type { Metadata, Viewport } from "next";
import { Vollkorn, IBM_Plex_Mono, IBM_Plex_Sans_Condensed } from "next/font/google";
import "./globals.css";
import { Providers } from "@/components/providers";
import { DotGrid } from "@/components/depth/DotGrid";

// Axis 2 type roles, loaded as CSS variables the token file reads.
const vollkorn = Vollkorn({
  subsets: ["latin"],
  weight: ["400", "500", "600", "700"],
  variable: "--font-vollkorn",
  display: "swap",
});
const plexMono = IBM_Plex_Mono({
  subsets: ["latin"],
  weight: ["400", "500", "600"],
  variable: "--font-plex-mono",
  display: "swap",
});
const plexSans = IBM_Plex_Sans_Condensed({
  subsets: ["latin"],
  weight: ["400", "500", "600", "700"],
  variable: "--font-plex-sans",
  display: "swap",
});

export const metadata: Metadata = {
  title: "Theorems Harness Console",
  description: "The developer control surface for the programmable Theorems agent harness.",
};

export const viewport: Viewport = {
  themeColor: "#ffffff",
  width: "device-width",
  initialScale: 1,
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html
      lang="en"
      data-theme="light"
      className={`${vollkorn.variable} ${plexMono.variable} ${plexSans.variable}`}
      suppressHydrationWarning
    >
      <head>
        {/* Apply the saved theme before first paint so dark-mode users never see
            a light flash (FOUC). Static literal, no user input. */}
        <script
          dangerouslySetInnerHTML={{
            __html: `try{var t=localStorage.getItem("harness-console-theme");if(t){document.documentElement.dataset.theme=t;}}catch(e){}`,
          }}
        />
      </head>
      <body className="font-sans antialiased">
        <a
          href="#main-content"
          className="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-[100] focus:rounded-md focus:border focus:border-line focus:bg-bg focus:px-3 focus:py-2 focus:text-body focus:text-ink"
        >
          Skip to content
        </a>
        {/* Depth Layer 1: the ambient field, behind everything. */}
        <DotGrid />
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
