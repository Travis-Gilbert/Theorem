/**
 * Vanilla 2D-canvas renderer for a SceneOS scene package.
 *
 * Lane B slice 1: NOT the React `AtomSubstrate` (cosmos.gl force engine). This
 * is a self-contained canvas renderer that takes the scene package Lane A
 * produced, runs the projection to place the atoms, and draws them — one file
 * Servo serves, no SPA, no WebGL.
 *
 * It draws the substrate's established vocabulary (relations as lines, atoms as
 * kind-glyphs with labels; see `palette.ts`) so the page is continuous with
 * the substrate's `genericTerminalState` and the browser's SERP chrome. Camera
 * fits the whole scene to the viewport; hover hit-tests real atom positions
 * (no theater).
 *
 * Canvas discipline (project rule): guard against zero dimensions and cap to
 * 8192px; scale the backing store by devicePixelRatio and draw in CSS pixels.
 *
 * Security: atom labels / kinds may originate from crawled pages or agent
 * output and are untrusted. Every piece of DOM text is written via
 * `textContent` / `createElement` (never `innerHTML`), mirroring the SERP page.
 */

import type { Atom } from '../atoms/types';
import type { ScenePackageV2 } from '../v2-package';
import {
  ACCENT,
  INK,
  INK_SOFT,
  MONO,
  PAPER,
  atomFill,
  relationStroke,
  shapeForGlyph,
  type ShapeKind,
} from './palette';
import {
  fitTransform,
  layoutScene,
  type FitTransform,
  type SceneLayout,
  type Viewport,
} from './sceneGeometry';

const MAX_CANVAS_PX = 8192;
const FIT_PADDING_PX = 64;
const MIN_ATOM_RADIUS = 5;
const MAX_ATOM_RADIUS = 18;
const LABEL_FONT_PX = 11;
/** Above this atom count, labels render only for the hovered atom to avoid
 *  an unreadable wall of text. */
const LABEL_ALL_THRESHOLD = 48;

interface PlacedAtom {
  atom: Atom;
  sx: number;
  sy: number;
  r: number;
  fill: string;
  shape: ShapeKind;
}

export interface SceneRendererCallbacks {
  /** Called when the user clicks an atom (real selection, wired by the host
   *  shell — e.g. open evidence / ask follow-up). */
  onSelectAtom?(atom: Atom): void;
}

export class SceneRenderer {
  private readonly canvas: HTMLCanvasElement;
  private readonly ctx: CanvasRenderingContext2D;
  private readonly tooltip: HTMLElement | null;
  private readonly callbacks: SceneRendererCallbacks;
  private readonly pkg: ScenePackageV2;
  private readonly reducedMotion: boolean;

  private layout: SceneLayout | null = null;
  private transform: FitTransform | null = null;
  private placed: PlacedAtom[] = [];
  private hoveredId: string | null = null;
  private viewport: Viewport = { width: 0, height: 0 };
  private fadeStart = 0;
  private rafId = 0;

  constructor(
    canvas: HTMLCanvasElement,
    pkg: ScenePackageV2,
    options: { tooltip?: HTMLElement | null; callbacks?: SceneRendererCallbacks } = {},
  ) {
    this.canvas = canvas;
    const ctx = canvas.getContext('2d');
    if (ctx === null) {
      throw new Error('SceneRenderer: 2D canvas context unavailable');
    }
    this.ctx = ctx;
    this.tooltip = options.tooltip ?? null;
    this.callbacks = options.callbacks ?? {};
    this.pkg = pkg;
    this.reducedMotion =
      typeof window !== 'undefined' && typeof window.matchMedia === 'function'
        ? window.matchMedia('(prefers-reduced-motion: reduce)').matches
        : false;

    this.onPointerMove = this.onPointerMove.bind(this);
    this.onPointerLeave = this.onPointerLeave.bind(this);
    this.onClick = this.onClick.bind(this);
    canvas.addEventListener('pointermove', this.onPointerMove);
    canvas.addEventListener('pointerleave', this.onPointerLeave);
    canvas.addEventListener('click', this.onClick);
  }

  /** The resolved layout (for the host shell to read projection id / fallback
   *  state and render an honest header / note). Null until first `resize`. */
  getLayout(): SceneLayout | null {
    return this.layout;
  }

  /**
   * Measure the canvas's CSS box, re-fit the scene, and paint. Call on mount
   * and whenever the container resizes. Re-runs the (pure) projection because
   * viewport-aware adapters depend on the available space.
   */
  resize(): void {
    const rect = this.canvas.getBoundingClientRect();
    const cssW = Math.floor(rect.width);
    const cssH = Math.floor(rect.height);
    // Guard zero dimensions (browsers show a broken-image icon) and cap to the
    // canvas size limit.
    if (cssW < 1 || cssH < 1) return;
    const w = Math.min(cssW, MAX_CANVAS_PX);
    const h = Math.min(cssH, MAX_CANVAS_PX);

    const dpr = Math.max(1, Math.min(3, window.devicePixelRatio || 1));
    this.canvas.width = Math.round(w * dpr);
    this.canvas.height = Math.round(h * dpr);
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    this.viewport = { width: w, height: h };
    this.layout = layoutScene(this.pkg, this.viewport);
    this.transform = fitTransform(this.layout.bounds, this.viewport, FIT_PADDING_PX);
    this.computePlaced();

    this.fadeStart = now();
    this.startPaintLoop();
  }

  destroy(): void {
    if (this.rafId !== 0) cancelAnimationFrame(this.rafId);
    this.canvas.removeEventListener('pointermove', this.onPointerMove);
    this.canvas.removeEventListener('pointerleave', this.onPointerLeave);
    this.canvas.removeEventListener('click', this.onClick);
    if (this.tooltip) this.tooltip.style.display = 'none';
  }

  // ------------------------------------------------------------------------
  // Placement
  // ------------------------------------------------------------------------

  private computePlaced(): void {
    const layout = this.layout;
    const transform = this.transform;
    this.placed = [];
    if (layout === null || transform === null) return;

    // Radius scales with atom weight/scale, clamped to a legible band.
    const weights = layout.atoms.map((a) => atomMagnitude(a));
    const maxWeight = weights.reduce((m, v) => Math.max(m, v), 0) || 1;

    for (const atom of layout.atoms) {
      const world = layout.positions.get(atom.id);
      if (world === undefined) continue;
      const sx = transform.toScreenX(world.x);
      const sy = transform.toScreenY(world.y);
      const norm = atomMagnitude(atom) / maxWeight;
      const r = MIN_ATOM_RADIUS + norm * (MAX_ATOM_RADIUS - MIN_ATOM_RADIUS);
      this.placed.push({
        atom,
        sx,
        sy,
        r,
        fill: atomFill(atom.kind, atom.color),
        shape: shapeForGlyph(atom.glyph),
      });
    }
  }

  // ------------------------------------------------------------------------
  // Paint
  // ------------------------------------------------------------------------

  private startPaintLoop(): void {
    if (this.rafId !== 0) cancelAnimationFrame(this.rafId);
    const tick = (): void => {
      const alpha = this.reducedMotion ? 1 : easeInOut(clamp01((now() - this.fadeStart) / 280));
      this.paint(alpha);
      if (alpha < 1) {
        this.rafId = requestAnimationFrame(tick);
      } else {
        this.rafId = 0;
      }
    };
    tick();
  }

  /** Repaint immediately (used by hover so the highlight is responsive even
   *  after the enter fade has settled). */
  private repaint(): void {
    if (this.rafId !== 0) return; // a fade is already running; it will repaint
    this.paint(1);
  }

  private paint(alpha: number): void {
    const { ctx } = this;
    const { width, height } = this.viewport;
    const layout = this.layout;
    if (layout === null) return;

    ctx.fillStyle = PAPER;
    ctx.fillRect(0, 0, width, height);

    ctx.globalAlpha = alpha;
    this.paintRelations(layout);
    this.paintAtoms(layout);
    ctx.globalAlpha = 1;
  }

  private paintRelations(layout: SceneLayout): void {
    const transform = this.transform;
    if (transform === null) return;
    const { ctx } = this;
    const fade = ctx.globalAlpha;
    const byId = new Map(this.placed.map((p) => [p.atom.id, p]));

    ctx.lineWidth = 1.25;
    for (const rel of layout.relations) {
      const a = byId.get(rel.sourceId);
      const b = byId.get(rel.targetId);
      if (a === undefined || b === undefined) continue;
      const highlight =
        this.hoveredId !== null &&
        (rel.sourceId === this.hoveredId || rel.targetId === this.hoveredId);
      const stroke = relationStroke(rel.color);
      ctx.strokeStyle = highlight ? ACCENT : stroke;
      ctx.globalAlpha = (highlight ? 0.95 : 0.5) * fade;
      ctx.beginPath();
      ctx.moveTo(a.sx, a.sy);
      ctx.lineTo(b.sx, b.sy);
      ctx.stroke();
      drawArrowhead(ctx, a, b, highlight ? ACCENT : stroke);
    }
    ctx.globalAlpha = fade;
  }

  private paintAtoms(layout: SceneLayout): void {
    const { ctx } = this;
    const fade = ctx.globalAlpha;
    const labelAll = layout.atoms.length <= LABEL_ALL_THRESHOLD;

    for (const p of this.placed) {
      const hovered = p.atom.id === this.hoveredId;
      const radius = hovered ? p.r * 1.18 : p.r;
      ctx.globalAlpha = lifecycleAlpha(p.atom) * fade;

      drawGlyph(ctx, p.shape, p.sx, p.sy, radius, p.fill);
      ctx.lineWidth = hovered ? 2 : 1;
      ctx.strokeStyle = hovered ? ACCENT : INK;
      strokeGlyph(ctx, p.shape, p.sx, p.sy, radius);

      if (labelAll || hovered) {
        drawLabel(ctx, p, hovered);
      }
    }
    ctx.globalAlpha = fade;
  }

  // ------------------------------------------------------------------------
  // Interaction
  // ------------------------------------------------------------------------

  private hitTest(cssX: number, cssY: number): PlacedAtom | null {
    let best: PlacedAtom | null = null;
    let bestDist = Infinity;
    for (const p of this.placed) {
      const dx = cssX - p.sx;
      const dy = cssY - p.sy;
      const dist = dx * dx + dy * dy;
      const hitR = p.r + 5;
      if (dist <= hitR * hitR && dist < bestDist) {
        best = p;
        bestDist = dist;
      }
    }
    return best;
  }

  private onPointerMove(event: PointerEvent): void {
    const rect = this.canvas.getBoundingClientRect();
    const cssX = event.clientX - rect.left;
    const cssY = event.clientY - rect.top;
    const hit = this.hitTest(cssX, cssY);
    const nextId = hit?.atom.id ?? null;
    this.canvas.style.cursor = hit ? 'pointer' : 'default';

    if (nextId !== this.hoveredId) {
      this.hoveredId = nextId;
      this.repaint();
    }
    this.showTooltip(hit, cssX, cssY);
  }

  private onPointerLeave(): void {
    if (this.hoveredId !== null) {
      this.hoveredId = null;
      this.repaint();
    }
    if (this.tooltip) this.tooltip.style.display = 'none';
    this.canvas.style.cursor = 'default';
  }

  private onClick(event: PointerEvent): void {
    const rect = this.canvas.getBoundingClientRect();
    const hit = this.hitTest(event.clientX - rect.left, event.clientY - rect.top);
    if (hit && this.callbacks.onSelectAtom) {
      this.callbacks.onSelectAtom(hit.atom);
    }
  }

  private showTooltip(hit: PlacedAtom | null, cssX: number, cssY: number): void {
    const tip = this.tooltip;
    if (tip === null) return;
    if (hit === null) {
      tip.style.display = 'none';
      return;
    }
    const atom = hit.atom;
    const refs = atom.sourceRefs?.length ?? 0;
    // Clear + rebuild with safe DOM nodes only (labels are untrusted; never
    // innerHTML). replaceChildren() empties the node without parsing HTML.
    tip.replaceChildren();
    const title = document.createElement('div');
    title.className = 'tip-title';
    title.textContent = atom.label ?? atom.id;
    tip.appendChild(title);
    const kind = document.createElement('div');
    kind.className = 'tip-kind';
    kind.textContent = atom.kind;
    tip.appendChild(kind);
    if (refs > 0) {
      const src = document.createElement('div');
      src.className = 'tip-src';
      src.textContent = `${refs} source${refs === 1 ? '' : 's'}`;
      tip.appendChild(src);
    }
    tip.style.display = 'block';
    // Keep the tooltip inside the viewport.
    const offset = 14;
    const tipW = tip.offsetWidth;
    const tipH = tip.offsetHeight;
    let left = cssX + offset;
    let top = cssY + offset;
    if (left + tipW > this.viewport.width) left = cssX - tipW - offset;
    if (top + tipH > this.viewport.height) top = cssY - tipH - offset;
    tip.style.left = `${Math.max(0, left)}px`;
    tip.style.top = `${Math.max(0, top)}px`;
  }
}

// --------------------------------------------------------------------------
// Glyph drawing
// --------------------------------------------------------------------------

function drawGlyph(
  ctx: CanvasRenderingContext2D,
  shape: ShapeKind,
  x: number,
  y: number,
  r: number,
  fill: string,
): void {
  ctx.fillStyle = fill;
  glyphPath(ctx, shape, x, y, r);
  ctx.fill();
}

function strokeGlyph(
  ctx: CanvasRenderingContext2D,
  shape: ShapeKind,
  x: number,
  y: number,
  r: number,
): void {
  glyphPath(ctx, shape, x, y, r);
  ctx.stroke();
}

/** Build the path for a glyph shape centered at (x, y) with radius r. Shapes
 *  echo `defaultGlyphForKind`: a source reads as a page, evidence as a pin,
 *  a cluster as a hex, etc. Unknown -> circle (the substrate baseline mark). */
function glyphPath(
  ctx: CanvasRenderingContext2D,
  shape: ShapeKind,
  x: number,
  y: number,
  r: number,
): void {
  ctx.beginPath();
  switch (shape) {
    case 'page': {
      const w = r * 1.5;
      const h = r * 1.9;
      ctx.rect(x - w / 2, y - h / 2, w, h);
      break;
    }
    case 'square': {
      const s = r * 1.7;
      ctx.rect(x - s / 2, y - s / 2, s, s);
      break;
    }
    case 'frame': {
      const w = r * 2.1;
      const h = r * 1.5;
      ctx.rect(x - w / 2, y - h / 2, w, h);
      break;
    }
    case 'building': {
      const w = r * 1.7;
      const h = r * 2.1;
      ctx.rect(x - w / 2, y - h / 2, w, h);
      break;
    }
    case 'pin': {
      // Teardrop: a circle with a point at the bottom.
      ctx.moveTo(x, y + r * 1.4);
      ctx.quadraticCurveTo(x - r, y + r * 0.2, x - r, y - r * 0.2);
      ctx.arc(x, y - r * 0.2, r, Math.PI, 0, false);
      ctx.quadraticCurveTo(x + r, y + r * 0.2, x, y + r * 1.4);
      break;
    }
    case 'person': {
      ctx.arc(x, y - r * 0.4, r * 0.6, 0, Math.PI * 2);
      ctx.moveTo(x + r * 0.9, y + r * 1.1);
      ctx.arc(x, y + r * 0.5, r * 0.9, 0, Math.PI, false);
      break;
    }
    case 'hex': {
      for (let i = 0; i < 6; i += 1) {
        const angle = (Math.PI / 3) * i - Math.PI / 6;
        const px = x + r * Math.cos(angle);
        const py = y + r * Math.sin(angle);
        if (i === 0) ctx.moveTo(px, py);
        else ctx.lineTo(px, py);
      }
      ctx.closePath();
      break;
    }
    case 'circle':
    default:
      ctx.arc(x, y, r, 0, Math.PI * 2);
      break;
  }
}

function drawArrowhead(
  ctx: CanvasRenderingContext2D,
  from: PlacedAtom,
  to: PlacedAtom,
  color: string,
): void {
  const dx = to.sx - from.sx;
  const dy = to.sy - from.sy;
  const len = Math.hypot(dx, dy);
  if (len < to.r + 6) return; // too short to bother
  const ux = dx / len;
  const uy = dy / len;
  // Land the arrow just outside the target glyph.
  const tipX = to.sx - ux * (to.r + 2);
  const tipY = to.sy - uy * (to.r + 2);
  const size = 5;
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.moveTo(tipX, tipY);
  ctx.lineTo(tipX - ux * size - uy * size * 0.6, tipY - uy * size + ux * size * 0.6);
  ctx.lineTo(tipX - ux * size + uy * size * 0.6, tipY - uy * size - ux * size * 0.6);
  ctx.closePath();
  ctx.fill();
}

function drawLabel(ctx: CanvasRenderingContext2D, p: PlacedAtom, hovered: boolean): void {
  const text = truncate(p.atom.label ?? p.atom.id, hovered ? 48 : 22);
  ctx.font = `${LABEL_FONT_PX}px ${MONO}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  const y = p.sy + p.r + 4;
  // Soft paper halo so labels stay legible over relation lines.
  const w = ctx.measureText(text).width;
  ctx.fillStyle = 'rgba(244, 241, 234, 0.82)';
  ctx.fillRect(p.sx - w / 2 - 2, y - 1, w + 4, LABEL_FONT_PX + 3);
  ctx.fillStyle = hovered ? INK : INK_SOFT;
  ctx.fillText(text, p.sx, y);
}

// --------------------------------------------------------------------------
// Small helpers
// --------------------------------------------------------------------------

function atomMagnitude(atom: Atom): number {
  if (typeof atom.weight === 'number' && Number.isFinite(atom.weight) && atom.weight > 0) {
    return atom.weight;
  }
  if (typeof atom.scale === 'number' && Number.isFinite(atom.scale) && atom.scale > 0) {
    return atom.scale;
  }
  return 1;
}

function lifecycleAlpha(atom: Atom): number {
  switch (atom.lifecycle) {
    case 'leaving':
      return 0.35;
    case 'entering':
      return 0.85;
    default:
      return 1;
  }
}

function truncate(value: string, max: number): string {
  const s = String(value);
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

function clamp01(value: number): number {
  if (value < 0) return 0;
  if (value > 1) return 1;
  return value;
}

function easeInOut(t: number): number {
  return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
}

function now(): number {
  return typeof performance !== 'undefined' && typeof performance.now === 'function'
    ? performance.now()
    : 0;
}
