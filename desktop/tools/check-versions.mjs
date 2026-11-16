// Fails when the app versions disagree. Run before tagging: node desktop/tools/check-versions.mjs
// CI runs it with the pushed tag: node desktop/tools/check-versions.mjs v0.2.0
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const read = (rel) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8');

// First `version = "..."` line after the [package] header (stops at the next section).
const cargoVersion = (text) => text.split(/^\[package\]\s*$/m)[1]?.split(/^\[/m)[0].match(/^version\s*=\s*"([^"]+)"/m)?.[1];

const tag = process.argv[2]?.replace(/^v/, '');
const found = [
  ['tauri.conf.json', JSON.parse(read('../src-tauri/tauri.conf.json')).version],
  ['desktop Cargo.toml', cargoVersion(read('../src-tauri/Cargo.toml'))],
  ['app Cargo.toml', cargoVersion(read('../../app/Cargo.toml'))],
];
if (tag !== undefined) found.push(['tag', tag]);

const line = [...found.map(([name, v]) => `${name} ${v ?? '?'}`), ...(tag === undefined ? ['tag –'] : [])].join(' · ');
if (found.some(([, v]) => !v) || new Set(found.map(([, v]) => v)).size > 1) {
  console.error(`Versions disagree: ${line}`);
  process.exit(1);
}
console.log(line);
