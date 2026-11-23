// WCAG 2 contrast check of the Tiffin token pairs in src/app.css. Run after any token change: node desktop/tools/contrast-check.mjs
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const css = readFileSync(fileURLToPath(new URL('../src/app.css', import.meta.url)), 'utf8');

// Value of every `--name: #hex` inside one block of text.
const hexes = (block) => Object.fromEntries([...block.matchAll(/(--[\w-]+):\s*(#[0-9a-f]{6})\b/gi)].map((m) => [m[1], m[2]]));

// The light :root block runs to the first closing brace; the dark one is the :root[data-theme='dark'] block.
const lightBlock = css.slice(css.indexOf(':root {'), css.indexOf(":root[data-theme='dark'] {"));
const darkBlock = css.slice(css.indexOf(":root[data-theme='dark'] {"), css.indexOf('* { box-sizing'));
const light = hexes(lightBlock);
const dark = { ...light, ...hexes(darkBlock) }; // dark falls back to light for tokens it does not set

// `--name: color-mix(in srgb, var(--a), var(--b) N%)` is resolved per mode from the hex tokens (sRGB, gamma-encoded, as CSS does).
const mixes = [...lightBlock.matchAll(/(--[\w-]+):\s*color-mix\(in srgb,\s*var\((--[\w-]+)\),\s*var\((--[\w-]+)\)\s*(\d+)%\)/g)];
for (const tokens of [light, dark]) {
  for (const [, name, a, b, pct] of mixes) {
    const at = (hex, i) => parseInt(hex.slice(i, i + 2), 16);
    const mixed = [1, 3, 5].map((i) => Math.round(at(tokens[a], i) * (1 - pct / 100) + at(tokens[b], i) * (pct / 100)));
    tokens[name] = '#' + mixed.map((v) => v.toString(16).padStart(2, '0')).join('');
  }
}

// WCAG 2 relative luminance of an sRGB colour.
const channel = (v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
const luminance = (hex) => {
  const [r, g, b] = [1, 3, 5].map((i) => channel(parseInt(hex.slice(i, i + 2), 16) / 255));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a, b) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

const TOAST = '#2b2622'; // the toast is dark in both modes
const WHITE = '#ffffff';
const text = 4.5;
const mark = 3;

// [foreground, background, minimum, modes]; names starting with -- are tokens, others literal colours.
const pairs = [];
const add = (fg, bgs, min, modes = ['light', 'dark']) => bgs.forEach((bg) => pairs.push([fg, bg, min, modes]));
add('--ink', ['--page', '--card', '--well', '--sidebar'], text);
add('--ink-2', ['--page', '--well', '--sidebar', '--accent-wash'], text);
add('--ink-3', ['--page', '--well', '--sidebar', '--accent-wash'], text);
add('--accent-text', ['--page', '--card', '--well', '--sidebar', '--accent-wash'], text);
add(WHITE, ['--accent', '--accent-press'], text);
add('--olive-text', ['--page', '--sidebar'], text);
add('--warn', ['--warn-wash'], text);
add('--heart', ['--page'], text);
add(WHITE, ['--heart'], text, ['light']);
add('--focus-ring', ['--page'], mark);
add('--brand', ['--page', '--sidebar'], mark);
add('--toast-act', [TOAST], text);
add('--toast-ok', [TOAST], text);
add('--targets-ink', ['--page'], text);
add('--body-ink', ['--page'], text);
add(WHITE, ['--tile-targets'], mark);
add(WHITE, ['--tile-body'], mark);
add('--home-ink', ['--page'], text);
add(WHITE, ['--tile-home'], mark);
add('--reports-ink', ['--page'], text);
add(WHITE, ['--tile-reports'], mark);

let failed = false;
for (const [fg, bg, min, modes] of pairs) {
  for (const mode of modes) {
    const tokens = mode === 'light' ? light : dark;
    const f = fg.startsWith('--') ? tokens[fg] : fg;
    const b = bg.startsWith('--') ? tokens[bg] : bg;
    if (!f || !b) {
      console.log(`${mode.padEnd(5)} ${fg} on ${bg}: FAIL (token not found)`);
      failed = true;
      continue;
    }
    const r = ratio(f, b);
    const ok = r >= min;
    if (!ok) failed = true;
    console.log(`${mode.padEnd(5)} ${fg} on ${bg}: ${r.toFixed(2)} (needs ${min}) ${ok ? 'PASS' : 'FAIL'}`);
  }
}
process.exit(failed ? 1 : 0);
