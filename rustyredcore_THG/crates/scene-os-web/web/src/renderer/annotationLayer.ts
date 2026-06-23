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

/** A model-explanation callout (phase 3): a short title plus the wrapped body
 *  (the model's explanation), anchored at a node's screen position. */
export interface AnnotationCallout {
  title: string;
  body: string;
  x: number;
  y: number;
}

/**
 * Controls the explanation overlay. Two independent callout layers share the
 * overlay `<svg>`: the SELECTION callout (one atom the user clicked) and the
 * MODEL callouts (phase 3, the GL-Fusion explanations pinned to anchor nodes).
 * They live in separate `<g>` groups so dropping a selection never wipes the
 * model explanations, and vice versa.
 */
export class AnnotationLayer {
  private readonly svg: SVGSVGElement;
  private readonly getViewport: () => AnnotationViewport;

  constructor(svg: SVGSVGElement, getViewport: () => AnnotationViewport) {
    this.svg = svg;
    this.getViewport = getViewport;
  }

  /**
   * Render a single selection callout for `atom` anchored at (screenX, screenY)
   * in the overlay's coordinate space. Replaces any prior selection callout but
   * leaves the model-explanation callouts intact.
   */
  showCallout(atom: Atom, screenX: number, screenY: number): void {
    const viewport = this.getViewport();
    // Lead the note toward whichever side has more room so it stays on-screen.
    const leadRight = screenX < viewport.width * 0.6;
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
      dx: leadRight ? 32 : -32,
      dy: screenY > viewport.height * 0.55 ? -28 : 28,
      id: atom.id,
    };
    this.renderInto('scene-annotation-selection', [spec]);
  }

  /**
   * Phase 3: render the model-explanation callouts, each pinned to its anchor
   * node's screen position. Persists across selection changes (own group).
   */
  showAnnotations(items: AnnotationCallout[]): void {
    const viewport = this.getViewport();
    const specs: AnnotationSpec[] = items.map((item) => {
      const leadRight = item.x < viewport.width * 0.6;
      return {
        note: {
          title: item.title,
          label: item.body,
          wrap: 220,
          padding: 6,
          align: leadRight ? 'left' : 'right',
          orientation: 'leftRight',
        },
        x: item.x,
        y: item.y,
        dx: leadRight ? 44 : -44,
        dy: item.y > viewport.height * 0.55 ? -40 : 40,
      };
    });
    this.renderInto('scene-annotation-model', specs);
  }

  /** Remove the selection callout only (selection dropped); model callouts stay. */
  clear(): void {
    this.removeGroup('scene-annotation-selection');
  }

  /** Remove every callout (teardown). `replaceChildren()` empties the SVG
   *  without parsing any HTML (project rule: never innerHTML). */
  clearAll(): void {
    this.svg.replaceChildren();
  }

  /**
   * Mount `specs` into a fresh `<g>` of class `className`, replacing any prior
   * group of that class. The builder reads title/label via d3's `.text()`
   * (textContent), so untrusted labels cannot inject markup.
   */
  private renderInto(className: string, specs: AnnotationSpec[]): void {
    this.removeGroup(className);
    if (specs.length === 0) return;
    const builder = makeBuilder().type(calloutType).annotations(specs);
    const group = select(this.svg)
      .append('g')
      .attr('class', `scene-annotation ${className}`) as Selection<
      SVGGElement,
      unknown,
      null,
      undefined
    >;
    builder(group);
  }

  private removeGroup(className: string): void {
    select(this.svg).selectAll(`g.${className}`).remove();
  }
}
