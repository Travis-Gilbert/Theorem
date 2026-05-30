/**
 * Headless execution of the canvas DRAW path (no browser).
 *
 * Playwright/Chrome is unavailable in this environment (OS-dep install needs
 * sudo), so instead of a pixel screenshot this harness runs the real
 * SceneRenderer against a RECORDING 2D context + stub DOM. It proves the paint
 * path executes end-to-end without throwing and issues the expected canvas
 * operations (glyph paths, strokes, fills, labels, DPR transform, relation
 * segments). It does NOT prove visual appearance — that gap is named in the
 * report; the screenshot is a follow-up when a browser is available.
 *
 * Run: node build a CJS of entry.ts first, then `node test/render-harness.mjs`.
 */

// ---- Recording 2D context -------------------------------------------------
const calls = Object.create(null);
function rec(name) {
  return (...args) => {
    calls[name] = (calls[name] ?? 0) + 1;
    void args;
  };
}
const ctx = {
  // settable state properties
  fillStyle: '',
  strokeStyle: '',
  lineWidth: 1,
  font: '',
  textAlign: '',
  textBaseline: '',
  globalAlpha: 1,
  // recorded methods
  setTransform: rec('setTransform'),
  fillRect: rec('fillRect'),
  beginPath: rec('beginPath'),
  moveTo: rec('moveTo'),
  lineTo: rec('lineTo'),
  arc: rec('arc'),
  rect: rec('rect'),
  quadraticCurveTo: rec('quadraticCurveTo'),
  closePath: rec('closePath'),
  stroke: rec('stroke'),
  fill: rec('fill'),
  fillText: rec('fillText'),
  measureText: (t) => {
    calls.measureText = (calls.measureText ?? 0) + 1;
    return { width: String(t).length * 6 };
  },
};

// ---- Stub DOM -------------------------------------------------------------
function makeEl(id) {
  const children = [];
  return {
    id,
    style: {},
    className: '',
    textContent: '',
    children,
    offsetWidth: 80,
    offsetHeight: 40,
    getContext: () => ctx,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 1200, height: 600 }),
    addEventListener: () => {},
    removeEventListener: () => {},
    appendChild: (c) => children.push(c),
    replaceChildren: () => {
      children.length = 0;
    },
    get parentElement() {
      return makeEl(`${id}-parent`);
    },
  };
}

const elements = new Map();
for (const id of [
  'scene-canvas',
  'scene-tooltip',
  'scene-header',
  'scene-title',
  'scene-meta',
  'scene-note',
  'scene-empty',
]) {
  elements.set(id, makeEl(id));
}

let clock = 0;
globalThis.performance = { now: () => (clock += 1000) };
globalThis.requestAnimationFrame = (cb) => {
  cb(clock);
  return 1;
};
globalThis.cancelAnimationFrame = () => {};
globalThis.ResizeObserver = class {
  observe() {}
  disconnect() {}
};
globalThis.document = {
  readyState: 'complete',
  getElementById: (id) => elements.get(id) ?? null,
  createElement: (tag) => makeEl(`new-${tag}`),
  addEventListener: () => {},
};
globalThis.window = {
  devicePixelRatio: 2,
  matchMedia: () => ({ matches: false }),
  addEventListener: () => {},
  __SCENE_PACKAGE__: null,
};

// ---- Run ------------------------------------------------------------------
const { mount } = await import('../dist/entry-node.mjs');

let failures = 0;
function check(label, cond) {
  console.log(`  ${cond ? 'ok  ' : 'FAIL'} ${label}`);
  if (!cond) failures += 1;
}

const samplePkg = {
  version: 'scene-package-v2',
  id: 'p',
  manifestRef: 'm',
  atoms: [
    { id: 'claim', kind: 'claim', label: 'Conclusion', lifecycle: 'present', weight: 3 },
    { id: 'ev', kind: 'evidence', label: 'A study', lifecycle: 'present', weight: 2 },
    { id: 'src', kind: 'source', label: 'Journal', lifecycle: 'present' },
  ],
  relations: [
    { id: 'ev-claim', sourceId: 'ev', targetId: 'claim', kind: 'supports', lifecycle: 'present' },
    { id: 'src-ev', sourceId: 'src', targetId: 'ev', kind: 'supports', lifecycle: 'present' },
  ],
  projection: { id: 'tree_hierarchy' },
  chrome: { id: 'document_rail' },
  actions: [],
};

console.log('draw path');
let renderer = null;
let threw = false;
try {
  renderer = mount(samplePkg);
} catch (err) {
  threw = true;
  console.error(err);
}
check('mount did not throw', !threw);
check('mount returned a renderer', renderer !== null);
check('DPR transform applied (setTransform)', (calls.setTransform ?? 0) > 0);
check('background + label halos painted (fillRect)', (calls.fillRect ?? 0) > 0);
check('glyph paths built (arc or rect)', (calls.arc ?? 0) + (calls.rect ?? 0) > 0);
check('atoms stroked', (calls.stroke ?? 0) > 0);
check('atoms filled', (calls.fill ?? 0) > 0);
check('relation segments drawn (moveTo + lineTo)', (calls.moveTo ?? 0) > 0 && (calls.lineTo ?? 0) > 0);
check('labels drawn (fillText)', (calls.fillText ?? 0) > 0);

// Header reflects the resolved projection + counts.
check('header title set to projection label', elements.get('scene-title').textContent === 'Tree Hierarchy');
check('header meta names atom + relation counts', /3 atoms, 2 relations/.test(elements.get('scene-meta').textContent));

console.log('empty-state path');
const empty = elements.get('scene-empty');
empty.children.length = 0;
const r2 = mount(null);
check('mount(null) renders no renderer', r2 === null);
check('empty state populated honestly', empty.children.length >= 2);

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall draw-path checks passed');
console.log('recorded ops:', JSON.stringify(calls));
