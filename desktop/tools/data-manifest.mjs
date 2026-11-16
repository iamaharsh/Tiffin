// Data-safety check for the Tiffin migration: proves a data folder was not touched.
//   node desktop/tools/data-manifest.mjs snap <folder> <out.json>
//   node desktop/tools/data-manifest.mjs diff <a.json> <b.json> [--allow path,path]
// It only reads the folder it snaps and must never write inside it (snap refuses an out.json there).
// diff exits 1 when a file was removed or changed; `--allow` lists paths (or folders) whose changes are expected.
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, relative, resolve, sep } from 'node:path';

const walk = (dir, root, files) => {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, e.name);
    const rel = relative(root, full).split(sep).join('/');
    if (e.isDirectory()) {
      files[rel + '/'] = { size: 0, sha256: 'dir' }; // so a removed empty folder shows up too
      walk(full, root, files);
    } else if (e.isFile()) {
      const bytes = readFileSync(full);
      files[rel] = { size: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') };
    }
  }
  return files;
};

const [cmd, a, b, ...rest] = process.argv.slice(2);

if (cmd === 'snap' && a && b) {
  const root = resolve(a);
  const out = resolve(b);
  if (out === root || out.startsWith(root + sep)) { console.error('Refusing to write the manifest inside the folder being snapped.'); process.exit(2); }
  const files = walk(root, root, {});
  writeFileSync(out, JSON.stringify({ root, taken: new Date().toISOString(), files }, null, 2));
  console.log(`snapped ${Object.keys(files).length} entries from ${root}`);
} else if (cmd === 'diff' && a && b) {
  const allowIdx = rest.indexOf('--allow');
  const allow = allowIdx >= 0 ? (rest[allowIdx + 1] ?? '').split(',').filter(Boolean) : [];
  const before = JSON.parse(readFileSync(a, 'utf8')).files;
  const after = JSON.parse(readFileSync(b, 'utf8')).files;
  const allowed = (p) => allow.some((x) => p === x || p.startsWith(x.endsWith('/') ? x : x + '/'));
  const removed = Object.keys(before).filter((p) => !(p in after));
  const changed = Object.keys(before).filter((p) => p in after && before[p].sha256 !== after[p].sha256 && !allowed(p));
  const added = Object.keys(after).filter((p) => !(p in before));
  console.log(`removed ${removed.length} · changed ${changed.length} · added ${added.length}`);
  for (const [tag, list] of [['removed', removed], ['changed', changed], ['added', added]]) for (const p of list) console.log(`  ${tag}: ${p}`);
  process.exit(removed.length || changed.length ? 1 : 0);
} else {
  console.error('usage: data-manifest.mjs snap <folder> <out.json> | diff <a.json> <b.json> [--allow p1,p2]');
  process.exit(2);
}
