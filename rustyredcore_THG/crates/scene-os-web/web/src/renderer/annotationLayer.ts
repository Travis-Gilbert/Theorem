/**
 * d3-annotation explanation layer for the SceneOS renderer.
 *
 * Build-order step 4 of the browser plan: the callout / explanation layer that
 * sits ABOVE the canvas. The canvas draws atoms + relations; this SVG overlay
 * draws a single d3-annotation callout for the atom the user has selected,
 * naming it (title = label, subtitle = kind) with a short leader line. The
 * overlay is pointer-events:none so it never intercepts the canvas's own hover
 * / click hit-testing.
 *
 * Why d3-annotation and not hand-rolled SVG: the callout's note box, leader,
 * and connector geometry are exactly what d3-annotation renders, and it sets
 * all note text via d3's `.text()` (which writes `textContent`), so an atom
 * label that came from a crawled page or agent output cannot inject markup.
 * The controller never enables editMode, so no drag interaction is wired.
 *
 * The bundle resolves `d3-svg-annotation` to its self-contained build (see
 * `build.mjs` alias): that build inlines its own d3-selection, so it does not
 * collide with the renderer's d3-selection v3. The controller passes a v3
 * selection to the builder for mounting; the static (non-interactive) render
 * path only reads the selection's append / text / attr surface, which is
 * stable across the two copies.
 */

import { select, type Selection } from 'd3-selection';
import { annotation, annotationCallout } from 'd3-svg-annotation';

import type { Atom } from '../atoms/types';

/**
 * Minimal typed surface for the d3-annotation builder. The bundled
 * `d3-svg-annotation` types model the builder as a non-callable class, but the
 * builder is in fact a function-object that mounts when invoked against a d3
 * selection. This interface names only the methods this controller uses, so the
 * single cast at construction stays honest rather than leaking `any`.
 */
interface AnnotationSpec {
  note: {
    title: string;
    label: string;
    /** Note box width in px so long labels wrap instead of overflowing. */
    wrap?: number;
    /** Padding between the leader and the note text. */
    padding?: number;
    align?: 'left' | 'right' | 'middle' | 'dynamic';
    orientation?: 'topBottom' | 'leftRight' | 'fixed';
  };
  /** Anchor point in overlay-SVG coordinates (the atom's screen position). */
  x: number;
  y: number;
  /** Leader-line offset from the anchor to the note box. */
  dx: number;
  dy: number;
  /** The atom id, kept for provenance / debugging on the datum. */
  id?: string;
}

interface AnnotationBuilder {
  (group: Selection<SVGGElement, unknown, null, undefined>): void;
  type(t: unknown): AnnotationBuilder;
  annotations(specs: AnnotationSpec[]): AnnotationBuilder;
}

/** The d3-annotation factory, typed through the local callable surface. */
const makeBuilder = annotation as unknown as () => AnnotationBuilder;
const calloutType = annotationCallout as unknown;

export interface AnnotationViewport {
  width: number;
  height: number;
}

/**
 * Controls the single-callout explanation overlay. Construct once with the
 * overlay `<svg>` element and a viewport accessor; call `showCallout` when an
 * atom is selected and `clear` when the selection is dropped.
 */
export class AnnotationLayer {
  private readonly svg: SVGSVGElement;
  private readonly getViewport: () => AnnotationViewport;

  constructor(svg: SVGSVGElement, getViewport: () => AnnotationViewport) {
    this.svg = svg;
    this.getViewport = getViewport;
  }

  /**
   * Render a single callout for `atom` anchored at (screenX, screenY) in the
   * overlay's coordinate space (the same CSS-pixel space the canvas hover uses,
   * since the overlay is sized to the stage). Replaces any prior callout.
   */
  showCallout(atom: Atom, screenX: number, screenY: number): void {
    this.clear();
    const viewport = this.getViewport();

    // Lead the note toward whichever side has more room so it stays on-screen.
    const leadRight = screenX < viewport.width * 0.6;
    const dx = leadRight ? 32 : -32;
    const dy = screenY > viewport.height * 0.55 ? -28 : 28;

    const spec: AnnotationSpec = {
      note: {
        title: atom.label ?? atom.id,
        label: atom.kind,
        wrap: 180,
        padding: 4,
        align: leadRight ? 'left' : 'right',
        orientation: 'leftRight',
      },
      x: screenX,
      y: screenY,
      dx,
      dy,
      id: atom.id,
    };

    const builder = makeBuilder().type(calloutType).annotations([spec]);

    // Mount into a fresh <g>. The builder reads title/label via d3's .text()
    // (textContent), so the untrusted atom label cannot inject markup.
    const group = select(this.svg)
      .append('g')
      .attr('class', 'scene-annotation') as Selection<
      SVGGElement,
      unknown,
      null,
      undefined
    >;
    builder(group);
  }

  /** Remove the current callout. `replaceChildren()` empties the SVG without
   *  parsing any HTML (project rule: never innerHTML). */
  clear(): void {
    this.svg.replaceChildren();
  }
}
