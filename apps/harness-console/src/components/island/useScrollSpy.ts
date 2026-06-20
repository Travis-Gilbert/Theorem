"use client";

import { useEffect, useRef } from "react";
import { useConsole, type TocEntry } from "./console-context";

/**
 * Convenience for content surfaces: track the TOC against the shell's content
 * well (#content-well) without threading a ref. Call once at the top of a
 * content page; it wires scroll-spy to the well so the Dynamic Island ambient
 * pill shows the active section with a progress ring.
 */
export function usePageToc() {
  const ref = useRef<HTMLElement | null>(null);
  useEffect(() => {
    ref.current = document.getElementById("content-well");
  }, []);
  useScrollSpy(ref);
}

/**
 * Scroll-spy for content surfaces. Reads headings tagged with data-toc /
 * data-toc-title / data-toc-depth (data-toc-ignore opts out), registers them as
 * the TOC, and tracks the active section + scroll progress so the ambient pill
 * shows the active section with a progress ring. Pass the scroll container ref.
 */
export function useScrollSpy(containerRef: React.RefObject<HTMLElement | null>) {
  const { setToc, setActiveSection, setProgress, setSurfaceMode } = useConsole();

  useEffect(() => {
    const root = containerRef.current;
    if (!root) return;
    setSurfaceMode("content");

    const nodes = Array.from(root.querySelectorAll<HTMLElement>("[data-toc]")).filter(
      (n) => n.dataset.tocIgnore == null,
    );
    const entries: TocEntry[] = nodes.map((n) => ({
      id: n.id || n.dataset.toc || "",
      title: n.dataset.tocTitle || n.textContent?.trim() || "",
      depth: Number(n.dataset.tocDepth ?? 1),
    }));
    setToc(entries);

    const io = new IntersectionObserver(
      (obs) => {
        const visible = obs
          .filter((o) => o.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0];
        if (visible) setActiveSection((visible.target as HTMLElement).id || null);
      },
      { root, rootMargin: "-20% 0px -70% 0px", threshold: 0 },
    );
    nodes.forEach((n) => io.observe(n));

    const onScroll = () => {
      const max = root.scrollHeight - root.clientHeight;
      setProgress(max > 0 ? Math.min(1, root.scrollTop / max) : 0);
    };
    root.addEventListener("scroll", onScroll, { passive: true });
    onScroll();

    return () => {
      io.disconnect();
      root.removeEventListener("scroll", onScroll);
      setToc([]);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [containerRef]);
}
