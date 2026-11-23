// Checks that the sidebar's tabs agree in all three places: the crate's TABS (app/src/sidebar.rs, which validates what
// is saved), the page's tabNames (src/lib/api.ts) and the Sidebar's own list (src/lib/Sidebar.svelte), same ids in the
// same default order. Run: node tools/sidebar-check.mjs (part of npm run check:ui).
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const read = (rel) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8');
const crate = [...(read('../../app/src/sidebar.rs').match(/pub const TABS: \[&str; \d+\] = \[([^\]]*)\]/)?.[1] ?? '').matchAll(/"([a-z]+)"/g)].map((m) => m[1]);
const names = [...(read('../src/lib/api.ts').match(/export const tabNames[^=]*= \{([^}]*)\}/)?.[1] ?? '').matchAll(/([a-z]+):\s*'/g)].map((m) => m[1]);
const sidebar = [...(read('../src/lib/Sidebar.svelte').match(/const sections: \[Section, string, string\]\[\] = \[([\s\S]*?)\];/)?.[1] ?? '').matchAll(/\['([a-z]+)'/g)].map((m) => m[1]);

let failed = false;
const check = (name, ok, detail) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${ok ? '' : `: ${detail}`}`); failed ||= !ok; };
check('the crate lists the tabs', crate.length > 0, 'TABS not found in app/src/sidebar.rs');
check('api.ts tabNames match the crate, in order', names.join() === crate.join(), `${names.join()} vs ${crate.join()}`);
check('Sidebar.svelte sections match the crate, in order', sidebar.join() === crate.join(), `${sidebar.join()} vs ${crate.join()}`);
console.log(failed ? 'sidebar-check: FAILED' : 'sidebar-check: all passed');
process.exit(failed ? 1 : 0);
