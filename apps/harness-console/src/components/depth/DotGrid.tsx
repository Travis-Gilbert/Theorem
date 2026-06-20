"use client";

import { useEffect, useRef } from "react";

/**
 * Depth Layer 1: the ambient field. A fixed full-viewport dot grid where a
 * seeded PRNG (mulberry32) deterministically turns about a fifth of the dots
 * into tiny 0s and 1s for a digital texture, with mouse repulsion, spring-back,
 * and a decaying ink trail. Recolored grey for the white scheme via CSS
 * variables (--dot-color, --dot-ink). inverseVignette keeps the work area clean
 * and fades dots in toward the edges. Reduced-motion renders the settled field
 * once and stops the loop.
 *
 * This is the single biggest depth win and it is the ported site technique,
 * re-tokenized rather than copied in look.
 */

function mulberry32(seed: number) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

interface Dot {
  x: number;
  y: number;
  bx: number; // base x
  by: number; // base y
  vx: number;
  vy: number;
  glyph: 0 | 1 | null; // null = plain dot; 0/1 = the PRNG binary scatter
}

const SPACING = 26;
const REPULSE_RADIUS = 110;

export function DotGrid() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const rng = mulberry32(0x504e524e); // "PRNG"
    let dots: Dot[] = [];
    let dpr = Math.min(window.devicePixelRatio || 1, 2);
    let raf = 0;

    const css = getComputedStyle(document.documentElement);
    const dotColor = (css.getPropertyValue("--dot-color").trim() || "148,148,154");
    const inkColor = (css.getPropertyValue("--dot-ink").trim() || "26,26,29");

    const mouse = { x: -9999, y: -9999 };

    function build() {
      const w = window.innerWidth;
      const h = window.innerHeight;
      canvas!.width = w * dpr;
      canvas!.height = h * dpr;
      canvas!.style.width = `${w}px`;
      canvas!.style.height = `${h}px`;
      ctx!.setTransform(dpr, 0, 0, dpr, 0, 0);
      dots = [];
      for (let y = SPACING; y < h; y += SPACING) {
        for (let x = SPACING; x < w; x += SPACING) {
          const r = rng();
          dots.push({
            x,
            y,
            bx: x,
            by: y,
            vx: 0,
            vy: 0,
            glyph: r < 0.2 ? (r < 0.1 ? 0 : 1) : null, // ~1/5 become 0s and 1s
          });
        }
      }
    }

    // inverseVignette: full transparency at center, fade dots IN toward edges so
    // the work area stays clean for reading.
    function edgeAlpha(x: number, y: number, w: number, h: number): number {
      const nx = Math.abs((x - w / 2) / (w / 2));
      const ny = Math.abs((y - h / 2) / (h / 2));
      const d = Math.max(nx, ny);
      return Math.min(1, Math.max(0, (d - 0.35) / 0.65)) * 0.9;
    }

    function frame() {
      const w = window.innerWidth;
      const h = window.innerHeight;
      ctx!.clearRect(0, 0, w, h);
      for (const d of dots) {
        if (!reduced) {
          const dx = d.x - mouse.x;
          const dy = d.y - mouse.y;
          const dist = Math.hypot(dx, dy);
          if (dist < REPULSE_RADIUS && dist > 0.01) {
            const force = (1 - dist / REPULSE_RADIUS) * 2.4;
            d.vx += (dx / dist) * force;
            d.vy += (dy / dist) * force;
          }
          // spring back to base
          d.vx += (d.bx - d.x) * 0.08;
          d.vy += (d.by - d.y) * 0.08;
          d.vx *= 0.82;
          d.vy *= 0.82;
          d.x += d.vx;
          d.y += d.vy;
        }

        const disp = Math.hypot(d.x - d.bx, d.y - d.by);
        const a = edgeAlpha(d.bx, d.by, w, h);
        const ink = disp > 1.5; // displaced dots flash the ink trail color
        const alpha = ink ? Math.min(0.5, a + disp / 40) : a;
        const color = ink ? inkColor : dotColor;

        if (d.glyph !== null) {
          ctx!.fillStyle = `rgba(${color}, ${alpha})`;
          ctx!.font = "9px var(--font-plex-mono, monospace)";
          ctx!.fillText(String(d.glyph), d.x - 2, d.y + 3);
        } else {
          ctx!.beginPath();
          ctx!.arc(d.x, d.y, 1, 0, Math.PI * 2);
          ctx!.fillStyle = `rgba(${color}, ${alpha})`;
          ctx!.fill();
        }
      }
      if (!reduced) raf = requestAnimationFrame(frame);
    }

    const onMove = (e: MouseEvent) => {
      mouse.x = e.clientX;
      mouse.y = e.clientY;
    };
    const onLeave = () => {
      mouse.x = -9999;
      mouse.y = -9999;
    };
    const onResize = () => {
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      build();
      if (reduced) frame();
    };

    build();
    frame();
    if (!reduced) {
      window.addEventListener("mousemove", onMove, { passive: true });
      window.addEventListener("mouseleave", onLeave);
    }
    window.addEventListener("resize", onResize);

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseleave", onLeave);
      window.removeEventListener("resize", onResize);
    };
  }, []);

  return (
    <canvas
      ref={canvasRef}
      aria-hidden
      className="pointer-events-none fixed inset-0 -z-10"
    />
  );
}
