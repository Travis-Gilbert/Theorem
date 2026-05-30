// Build the SceneOS renderer into a single self-contained IIFE bundle.
//
// The output (`dist/scene-os.bundle.js`) is embedded in the Rust crate via
// include_str! and injected into scene-host.html at serve time, mirroring how
// rustyred-web inlines vendored d3 into serp.html. One file, no CDN, no SPA:
// Servo serves it directly.
//
// d3-hierarchy / d3-scale / d3-sankey are bundled in (inlined), so there is no
// runtime module resolution in the browser.

import { build } from 'esbuild';
import { mkdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const outfile = join(here, 'dist', 'scene-os.bundle.js');

mkdirSync(join(here, 'dist'), { recursive: true });

// d3-svg-annotation ships two builds. Its ESM `module` build imports
// `{ event }` from d3-selection, a symbol d3-selection v3 removed, so bundling
// it against our d3-selection v3 fails. Its CJS `main` build (indexRollup.js)
// is fully self-contained (it inlines its own d3-selection 1.4.0, where `event`
// exists), so we alias the package to that build. The renderer never enables
// the annotation editMode, so the inlined drag/selection code stays inert.
const selfContainedAnnotation = join(
  here,
  'node_modules',
  'd3-svg-annotation',
  'indexRollup.js',
);

await build({
  entryPoints: [join(here, 'src', 'entry.ts')],
  bundle: true,
  format: 'iife',
  platform: 'browser',
  target: ['es2020'],
  minify: true,
  legalComments: 'none',
  alias: {
    'd3-svg-annotation': selfContainedAnnotation,
  },
  outfile,
  logLevel: 'info',
});

const { size } = statSync(outfile);
console.log(`scene-os.bundle.js: ${(size / 1024).toFixed(1)} KB`);
