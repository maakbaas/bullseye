// Builds the Carbon design system into src/vendor/, which is committed: the page loads these local
// files and has no bundler of its own, and the app needs no network. Run `npm run vendor` after
// changing scripts/carbon/ (another component, icon or style) or updating the Carbon packages.
import { build } from 'esbuild';
import * as sass from 'sass';
import { copyFileSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(root, 'src', 'vendor');
rmSync(out, { recursive: true, force: true });
mkdirSync(join(out, 'fonts'), { recursive: true });

await build({
  entryPoints: [join(root, 'scripts', 'carbon', 'entry.js')],
  bundle: true,
  minify: true,
  format: 'iife',
  target: 'es2022',
  legalComments: 'eof',
  outfile: join(out, 'carbon.js'),
});

const css = sass.compile(join(root, 'scripts', 'carbon', 'carbon.scss'), {
  loadPaths: [join(root, 'node_modules')],
  style: 'compressed',
  quietDeps: true,
  silenceDeprecations: ['import', 'global-builtin'],
});
writeFileSync(join(out, 'carbon.css'), css.css);

// IBM Plex: the weights Carbon uses (400 and 600), Sans for text and Mono for code.
const fonts = [
  ['IBM Plex Sans', 400, '@ibm/plex-sans/fonts/complete/woff2/IBMPlexSans-Regular.woff2'],
  ['IBM Plex Sans', 600, '@ibm/plex-sans/fonts/complete/woff2/IBMPlexSans-SemiBold.woff2'],
  ['IBM Plex Mono', 400, '@ibm/plex-mono/fonts/complete/woff2/IBMPlexMono-Regular.woff2'],
];
let faces = '';
for (const [family, weight, path] of fonts) {
  const file = path.split('/').pop();
  copyFileSync(join(root, 'node_modules', path), join(out, 'fonts', file));
  faces += `@font-face{font-family:"${family}";font-style:normal;font-weight:${weight};font-display:block;src:url("fonts/${file}") format("woff2")}\n`;
}
writeFileSync(join(out, 'fonts.css'), faces);
copyFileSync(join(root, 'node_modules', '@ibm/plex-sans/LICENSE.txt'), join(out, 'fonts', 'LICENSE.txt'));
console.log('vendored Carbon into', out);
