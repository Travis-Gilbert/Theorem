/**
 * The elevation scale as code: depth is quantized and checkable, never tuned by
 * hand. Each level maps to one shadow token (globals.css) and one z-index band.
 * Use `elevationClass(level)` so no surface invents a freehand shadow.
 */
export type Elevation = 0 | 1 | 2 | 3;

export const ELEVATION_LABELS: Record<Elevation, string> = {
  0: "flat", // on the field, no shadow
  1: "raised", // a card
  2: "floating", // the Dynamic Island and popovers
  3: "overlay", // modals and the expanded-TOC backdrop
};

export function elevationClass(level: Elevation): string {
  return `elev-${level}`;
}

export const Z = {
  rail: 20,
  topbar: 30,
  island: 60,
  overlay: 80,
} as const;
