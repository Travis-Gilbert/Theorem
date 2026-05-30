/**
 * Bundle entry for the Theorem SceneOS renderer.
 *
 * esbuild compiles this to a single IIFE (`dist/scene-os.bundle.js`) with d3
 * inlined, embedded in `scene-host.html`. Servo serves that one asset; the
 * Rust `render_scene_html` injects the scene package Lane A produced in place
 * of the `__SCENE_PACKAGE__` marker (the SERP injection pattern).
 *
 * Responsibilities:
 *   - Read the injected package (`window.__SCENE_PACKAGE__`), validate it.
 *   - Mount `SceneRenderer` into the canvas; wire the header, tooltip, and a
 *     ResizeObserver.
 *   - Render an HONEST empty state when there is no package / no atoms, and an
 *     honest fallback note when the projection id was unknown or placement was
 *     degenerate. No fake data, ever.
 *   - Expose `window.SceneOS.mount(pkg)` so a host (Lane C, or a test) can
 *     mount programmatically in addition to the injected global.
 *
 * All DOM text comes from `textContent` (atom labels are untrusted), never
 * innerHTML: same discipline as the SERP page.
 */

import { SceneRenderer } from './renderer/SceneRenderer';
import type { ScenePackageV2 } from './v2-package';
import { validateScenePackageV2 } from './v2-package';

interface MountHandles {
  canvas: HTMLCanvasElement;
  tooltip: HTMLElement | null;
  overlay: SVGSVGElement | null;
  header: HTMLElement | null;
  title: HTMLElement | null;
  meta: HTMLElement | null;
  note: HTMLElement | null;
  empty: HTMLElement | null;
}

let activeRenderer: SceneRenderer | null = null;
let activeObserver: ResizeObserver | null = null;

function handles(): MountHandles {
  return {
    canvas: document.getElementById('scene-canvas') as HTMLCanvasElement,
    tooltip: document.getElementById('scene-tooltip'),
    overlay: document.getElementById('scene-annotations') as SVGSVGElement | null,
    header: document.getElementById('scene-header'),
    title: document.getElementById('scene-title'),
    meta: document.getElementById('scene-meta'),
    note: document.getElementById('scene-note'),
    empty: document.getElementById('scene-empty'),
  };
}

function showEmpty(h: MountHandles, headline: string, detail: string): void {
  if (h.canvas) h.canvas.style.display = 'none';
  if (h.note) h.note.style.display = 'none';
  if (h.empty) {
    h.empty.replaceChildren();
    const strong = document.createElement('strong');
    strong.textContent = headline;
    const p = document.createElement('span');
    p.textContent = detail;
    h.empty.appendChild(strong);
    h.empty.appendChild(p);
    h.empty.style.display = 'flex';
  }
  if (h.title) h.title.textContent = 'No scene';
  if (h.meta) h.meta.textContent = '';
}

function setNote(noteEl: HTMLElement | null, text: string | null): void {
  if (noteEl === null) return;
  if (text === null) {
    noteEl.style.display = 'none';
    noteEl.textContent = '';
    return;
  }
  noteEl.textContent = text;
  noteEl.style.display = 'block';
}

/**
 * Mount a scene package into the host DOM. Returns the renderer, or null if the
 * package was unusable (an honest empty state is shown in that case).
 */
export function mount(pkg: unknown): SceneRenderer | null {
  const h = handles();
  if (!h.canvas) {
    // No host scaffold; nothing we can do.
    return null;
  }

  // Tear down any prior mount (re-injection / hot reload).
  if (activeRenderer) {
    activeRenderer.destroy();
    activeRenderer = null;
  }
  if (activeObserver) {
    activeObserver.disconnect();
    activeObserver = null;
  }

  if (pkg === null || pkg === undefined || typeof pkg !== 'object') {
    showEmpty(
      h,
      'No scene to render',
      'The browser served this page without a scene package. Run a query that produces a scene.',
    );
    return null;
  }

  const candidate = pkg as ScenePackageV2;
  const validationError = validateScenePackageV2(candidate);
  if (validationError !== null) {
    showEmpty(h, 'Scene package rejected', validationError);
    return null;
  }
  if (!Array.isArray(candidate.atoms) || candidate.atoms.length === 0) {
    showEmpty(
      h,
      'Empty scene',
      'The director produced a valid package with no atoms to place. Nothing to draw yet.',
    );
    return null;
  }

  if (h.empty) h.empty.style.display = 'none';
  h.canvas.style.display = 'block';

  const renderer = new SceneRenderer(h.canvas, candidate, {
    tooltip: h.tooltip,
    overlay: h.overlay,
    callbacks: {
      onSelectAtom: (atom) => {
        const refs = atom.sourceRefs?.length ?? 0;
        const label = atom.label ?? atom.id;
        setNote(
          h.note,
          refs > 0
            ? `Selected: ${label} (${refs} source${refs === 1 ? '' : 's'})`
            : `Selected: ${label}`,
        );
      },
    },
  });
  renderer.resize();
  activeRenderer = renderer;

  // Header: projection label + atom/relation counts, read from the resolved
  // layout so a fallback is named honestly.
  const layout = renderer.getLayout();
  if (h.title) {
    h.title.textContent = layout ? layout.projectionLabel : candidate.projection.id;
  }
  if (h.meta) {
    const atomCount = candidate.atoms.length;
    const relCount = candidate.relations.length;
    h.meta.textContent =
      `${atomCount} atom${atomCount === 1 ? '' : 's'}, ` +
      `${relCount} relation${relCount === 1 ? '' : 's'}`;
  }
  if (layout && layout.fellBack) {
    setNote(
      h.note,
      `Projection "${layout.requestedProjectionId}" is not available in this build: ` +
        `rendered in freeform space.`,
    );
  } else if (layout && layout.gridFallback) {
    setNote(h.note, 'No positions supplied; atoms arranged in a fallback grid.');
  } else {
    setNote(h.note, null);
  }

  // Keep the canvas fitted to its container.
  const container = h.canvas.parentElement ?? h.canvas;
  if (typeof ResizeObserver !== 'undefined') {
    activeObserver = new ResizeObserver(() => {
      if (activeRenderer) activeRenderer.resize();
    });
    activeObserver.observe(container);
  } else if (typeof window !== 'undefined') {
    window.addEventListener('resize', () => {
      if (activeRenderer) activeRenderer.resize();
    });
  }

  return renderer;
}

function boot(): void {
  const injected = (window as unknown as { __SCENE_PACKAGE__?: unknown }).__SCENE_PACKAGE__;
  mount(injected ?? null);
}

// Programmatic API for Lane C / tests.
(window as unknown as { SceneOS?: { mount: typeof mount } }).SceneOS = { mount };

if (typeof document !== 'undefined') {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
  } else {
    boot();
  }
}
