// Scripted check of Segmented.svelte: key map, reduced-motion rule and radiogroup markup.
// Run after any change to Segmented.svelte: node desktop/tools/segmented-check.mjs
import { readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';
import { createRawSnippet } from 'svelte';

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const src = readFileSync(here('../src/lib/Segmented.svelte'), 'utf8');
// The compiled module must sit under desktop/ so its `svelte` imports resolve.
const dir = here('../node_modules/.segmented-check/');

let failed = 0;
const check = (name, ok) => {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}`);
  if (!ok) failed++;
};
const count = (s, re) => (s.match(re) ?? []).length;

try {
  mkdirSync(dir, { recursive: true });
  const { js } = compile(src, { generate: 'server', filename: 'Segmented.svelte' });
  const file = dir + 'segmented.server.mjs';
  writeFileSync(file, js.code);
  const { default: Segmented, step, instant } = await import(pathToFileURL(file).href);

  const keyMap = [
    ['ArrowRight', 2, 3, 0], ['ArrowDown', 0, 3, 1], ['ArrowLeft', 0, 3, 2], ['ArrowUp', 1, 3, 0],
    ['Home', 1, 3, 0], ['End', 0, 3, 2], ['a', 0, 3, null], ['Tab', 0, 3, null],
    ['End', 0, 4, 3], ['ArrowRight', 3, 4, 0],
  ];
  for (const [key, i, n, want] of keyMap) check(`step(${key}, ${i}, ${n}) = ${want}`, step(key, i, n) === want);

  check('instant(false, false) is true (first paint)', instant(false, false) === true);
  check('instant(true, true) is true (reduced motion)', instant(true, true) === true);
  check('instant(true, false) is false (spring)', instant(true, false) === false);

  const item = createRawSnippet((o) => ({ render: () => `<span>${names[o()]}</span>` }));
  const names = ['Section', 'Added', 'Expires', 'In Stock'];
  for (const [options, value] of [[[0, 1, 2], 1], [[0, 1, 2, 3], 3]]) {
    names.splice(0, 4, ...(options.length === 3 ? ['Name', 'Rating', 'Newest', ''] : ['Section', 'Added', 'Expires', 'In Stock']));
    const { body } = render(Segmented, {
      props: { options, value, onchange() {}, label: 'Sort by', title: 'Sort by', status: (i) => 'Sorted by ' + names[i], item },
    });
    const n = options.length;
    const tag = `${n} segments`;
    check(`${tag}: one radiogroup named "Sort by"`, count(body, /role="radiogroup"/g) === 1 && body.includes('aria-label="Sort by"'));
    check(`${tag}: ${n} radios`, count(body, /role="radio"/g) === n);
    check(`${tag}: one checked, at position ${value}`, count(body, /aria-checked="true"/g) === 1 && count(body, /aria-checked="false"/g) === n - 1
      && new RegExp(`(role="radio"[^>]*aria-checked="false"[^>]*>[\\s\\S]*?){${value}}role="radio"[^>]*aria-checked="true"`).test(body));
    check(`${tag}: one Tab stop`, count(body, /tabindex="0"/g) === 1);
    check(`${tag}: polite status is empty on first render`, /role="status"[^>]*><\/span>/.test(body) && count(body, /role="status"/g) === 1);
    check(`${tag}: ${n} aria-hidden sizer copies`, count(body, /class="[^"]*sizer[^"]*"[^>]*aria-hidden="true"|aria-hidden="true"[^>]*class="[^"]*sizer/g) === n);
    check(`${tag}: every label shown`, names.slice(0, n).every((s) => body.includes(`<span>${s}</span>`)));
  }
} catch (e) {
  console.log('FAIL  ' + (e?.stack ?? e));
  failed++;
} finally {
  rmSync(dir, { recursive: true, force: true });
}

if (failed) {
  console.log(`${failed} check(s) failed`);
  process.exit(1);
}
console.log('segmented-check: all passed');
