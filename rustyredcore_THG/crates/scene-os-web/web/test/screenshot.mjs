/**
 * Optional visual check (browser screenshot). NOT part of the crate's required
 * tooling — playwright is intentionally not a committed dependency so the base
 * `npm install` stays lean (the build only needs esbuild). The headless logic
 * tests that DO run on every change are `smoke.ts` + `render-harness.mjs`.
 *
 * To run this:
 *   1. cargo run -p scene-os-web --example render_sample -- /tmp/scene-sample.html
 *   2. npm i -D playwright && npx playwright install chromium
 *   3. node test/screenshot.mjs [file:///tmp/scene-sample.html] [/tmp/scene.png]
 *
 * It loads the Rust-rendered page in headless chromium, waits a short beat
 * (the renderer paints synchronously, so the scene is up almost immediately —
 * no rAF-gated fade), screenshots it, and reports any console errors.
 */
import { chromium } from 'playwright';

const url = process.argv[2] ?? 'file:///tmp/scene-sample.html';
const out = process.argv[3] ?? '/tmp/scene.png';

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
const errors = [];
page.on('console', (m) => {
  if (m.type() === 'error') errors.push(m.text());
});
page.on('pageerror', (e) => errors.push(String(e)));

await page.goto(url, { waitUntil: 'domcontentloaded' });
await page.waitForTimeout(150);
await page.screenshot({ path: out });
await browser.close();

console.log(`screenshot -> ${out}`);
console.log(errors.length ? `console errors:\n${errors.join('\n')}` : 'no console errors');
