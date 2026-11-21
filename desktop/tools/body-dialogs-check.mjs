// Scripted check of the three body dialogs (Log Weight, Edit Measurements, Manage Measures): server-renders fixtures and checks
// that Edit Measurements starts from the stored numbers (never the rounded cell text), that Manage Measures offers archive and
// restore but no delete, and that only the Update Target handler can change the target. Run after any change to those files:
//   node desktop/tools/body-dialogs-check.mjs
import { readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const lib = (n) => readFileSync(here(`../src/lib/${n}.svelte`), 'utf8');
// The compiled modules must sit under desktop/ so their `svelte` imports resolve.
const dir = here('../node_modules/.body-dialogs-check/');

let failed = 0;
const check = (name, ok) => {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}`);
  if (!ok) failed++;
};
const count = (s, re) => (s.match(re) ?? []).length;

try {
  mkdirSync(dir, { recursive: true });
  // the dialogs call the Tauri bridge through api.ts; the check gives them a stand-in that never runs (a server render runs no effects)
  writeFileSync(dir + 'api-stub.mjs', 'export const reason = (e) => String(e);\nexport const bodyView = async () => ({});\nexport const logMeasure = async () => ({});\nexport const acceptTargetUpdate = async () => ({});\nexport const editMeasurements = async () => ({});\nexport const saveMeasure = async () => ({});\nexport const archiveMeasure = async () => ({});\n');
  const load = async (name) => {
    const code = lib(name).replace(/from '\.\/api'/g, "from './api-stub.mjs'").replace(/from '\.\/(\w+)\.svelte'/g, "from './$1.server.mjs'");
    writeFileSync(`${dir}${name}.server.mjs`, compile(code, { generate: 'server', filename: `${name}.svelte` }).js.code);
    return name;
  };
  for (const n of ['Dialog', 'Icon', 'Segmented', 'EditMeasurementsDialog', 'ManageMeasuresDialog', 'LogWeightDialog']) await load(n);
  const open = async (n) => (await import(pathToFileURL(`${dir}${n}.server.mjs`).href)).default;
  const noop = () => {};

  // Edit Measurements: 174.04 lb and 84.37 cm are what is stored; the table would show "174.0 lb" and "84.4 cm"
  const columns = [
    { id: 'weight', name: 'Weight', unit_text: 'lb' },
    { id: 'waist', name: 'Waist', unit_text: 'cm' },
    { id: 'chest', name: 'Chest', unit_text: 'cm' },
  ];
  const row = {
    day: '2026-11-15',
    date_text: 'Nov 15',
    cells: ['174.0 lb', '84.4 cm', null],
    values: [{ value: 174.04, unit: 'lb' }, { value: 84.37, unit: 'cm' }, null],
  };
  const edit = render(await open('EditMeasurementsDialog'), { props: { view: { columns, rows: [row], measures: [], archived: [] }, row, dateText: 'Nov 15', today: '2026-11-17', range: '3m', onclose: noop, onsaved: noop, onerror: noop } }).body;
  check('Edit Measurements prefills the exact stored weight, not the rounded cell text', edit.includes('value="174.04"') && !edit.includes('value="174"') && !edit.includes('value="174.0"'));
  check('Edit Measurements prefills the exact stored length', edit.includes('value="84.37"') && !edit.includes('value="84.4"'));
  check('Edit Measurements: a day with no chest value gives an empty box', count(edit, /<input\b/g) === 3 && count(edit, /value="[\d.]+"/g) === 2);
  check('Edit Measurements: one labelled box per column, the clear hint and both buttons', count(edit, /<label\b/g) === 3 && edit.includes('Clear a box and save to remove that value.') && edit.includes('Discard Edits') && edit.includes('Save Changes'));
  check('Edit Measurements titles the dialog with the day', edit.includes('Measurements for Nov 15'));

  // Manage Measures: eight built-ins, one of the person's own, one archived
  const builtIn = ['Weight', 'Body fat', 'Waist', 'Hips', 'Chest', 'Neck', 'Arms', 'Thighs'].map((name, i) => ({ id: `b${i}`, name, kind: i === 0 ? 'mass' : 'length', unit_text: 'cm' }));
  const own = { id: 'u1', name: 'Calf', kind: 'length', unit_text: 'cm' };
  const gone = { id: 'u2', name: 'Ankle', kind: 'length', unit_text: 'cm' };
  const manage = render(await open('ManageMeasuresDialog'), { props: { view: { measures: [...builtIn, own], archived: [gone] }, range: '3m', today: '2026-11-17', onclose: noop, onview: noop, onarchived: noop } }).body;
  check('Manage Measures: Archive Measure only on the person\'s own measure', count(manage, /Archive Measure<\/button>/g) === 1);
  check('Manage Measures: Restore Measure on the archived one, in a disclosure', count(manage, /Restore Measure<\/button>/g) === 1 && manage.includes('<details'));
  check('Manage Measures: Add Measure, Done Managing Measures, a 40-character name limit', manage.includes('Add Measure') && manage.includes('Done Managing Measures') && manage.includes('maxlength="40"'));
  check('Manage Measures: no delete or remove action anywhere', !/delete|remove/i.test(manage) && !/delete|remove/i.test(lib('ManageMeasuresDialog').replace(/\/\/.*|<!--[\s\S]*?-->/g, '')));

  // Log Weight: the first step offers no target change, and only updateTarget() can call acceptTargetUpdate
  const weight = render(await open('LogWeightDialog'), { props: { day: '2026-11-17', today: '2026-11-17', dateText: 'Tue, Nov 17', onclose: noop, onlogged: noop, onupdated: noop, onerror: noop } }).body;
  check('Log Weight names the day and offers Save Weight, not Update Target', weight.includes('For today') && weight.includes('Save Weight') && !weight.includes('Update Target'));
  const src = lib('LogWeightDialog');
  const body = (name) => src.match(new RegExp(`(?:async )?function ${name}\\(\\) \\{[\\s\\S]*?\\n  \\}`))?.[0] ?? '';
  check('acceptTargetUpdate is called once, inside updateTarget', count(src, /acceptTargetUpdate\(/g) === 1 && body('updateTarget').includes('acceptTargetUpdate('));
  check('Not Now and save never call acceptTargetUpdate', body('notNow') !== '' && !body('notNow').includes('acceptTargetUpdate') && !body('save').includes('acceptTargetUpdate'));
} finally {
  rmSync(dir, { recursive: true, force: true });
}
if (failed) {
  console.log(`body-dialogs-check: ${failed} failed`);
  process.exit(1);
}
console.log('body-dialogs-check: all passed');
