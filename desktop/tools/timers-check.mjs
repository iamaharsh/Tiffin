// Scripted check of timers.svelte.ts: countdown formatting and web tab-title states.
// Run after any change to timers.svelte.ts: node desktop/tools/timers-check.mjs
import { readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import ts from 'typescript';
import { compileModule } from 'svelte/compiler';

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const src = readFileSync(here('../src/lib/timers.svelte.ts'), 'utf8');
const dir = here('../node_modules/.timers-check/');

let failed = 0;
const check = (name, actual, expected) => {
  const ok = actual === expected;
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${ok ? '' : ` (got ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)})`}`);
  if (!ok) failed++;
};

try {
  mkdirSync(dir, { recursive: true });
  writeFileSync(dir + 'api-stub.mjs', 'export const isWeb = true;\n');
  const jsOnly = ts.transpileModule(src.replace(/from '\.\/api'/g, "from './api-stub.mjs'"), {
    compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext },
  }).outputText;
  const { js } = compileModule(jsOnly, { generate: 'server', filename: 'timers.svelte.js' });
  const file = dir + 'timers.server.mjs';
  writeFileSync(file, js.code);
  const { formatLeft, timerTitle } = await import(pathToFileURL(file).href);

  check('formatLeft(0)', formatLeft(0), '0:00');
  check('formatLeft(-2) clamps to 0:00', formatLeft(-2), '0:00');
  check('formatLeft(NaN) clamps to 0:00', formatLeft(Number.NaN), '0:00');
  check('formatLeft(5.1) rounds up', formatLeft(5.1), '0:06');
  check('formatLeft(59.1) rolls into 1:00', formatLeft(59.1), '1:00');
  check('formatLeft(65)', formatLeft(65), '1:05');
  check('formatLeft(3600)', formatLeft(3600), '1:00:00');
  check('formatLeft(3661)', formatLeft(3661), '1:01:01');

  const mk = (id, label, left, endsAt, done) => ({ id, label, total: 600, left, endsAt, done });
  check('empty list -> Tiffin', timerTitle([]), 'Tiffin');
  check('paused only -> Tiffin', timerTitle([mk(1, 'Pasta', 120, 0, false)]), 'Tiffin');
  check('single running -> countdown · Tiffin', timerTitle([mk(1, 'Pasta', 95, 9999, false)]), '1:35 · Tiffin');
  check('multiple running picks soonest', timerTitle([mk(1, 'Roast', 1800, 9999, false), mk(2, 'Sauce', 42, 9999, false)]), '0:42 · Tiffin');
  check('paused shorter timer ignored over running', timerTitle([mk(1, 'Short', 10, 0, false), mk(2, 'Long', 90, 9999, false)]), '1:30 · Tiffin');
  check('one done wins over running', timerTitle([mk(1, 'Roast', 300, 9999, false), mk(2, 'Pasta', 0, 0, true)]), 'Done · Pasta · Tiffin');
  check('one done with blank label -> Done · Tiffin', timerTitle([mk(1, '  ', 0, 0, true)]), 'Done · Tiffin');
  check('multiple done shows count', timerTitle([mk(1, 'Pasta', 0, 0, true), mk(2, 'Garlic', 0, 0, true)]), 'Done (2) · Tiffin');
} catch (e) {
  console.log('FAIL  ' + (e?.stack ?? e));
  failed++;
} finally {
  rmSync(dir, { recursive: true, force: true });
}

if (failed) {
  console.log(`\ntimers-check: ${failed} failed`);
  process.exit(1);
}
console.log('timers-check: all passed');
