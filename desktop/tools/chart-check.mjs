// Scripted check of LineChart.svelte: server-renders fixtures and checks the marks, the gaps, the goal line, the aria summary,
// table mode and that the source only uses scaleLinear from d3-scale. Run after any change to LineChart.svelte:
//   node desktop/tools/chart-check.mjs
import { readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const src = readFileSync(here('../src/lib/LineChart.svelte'), 'utf8');
// The compiled module must sit under desktop/ so its `svelte` and `d3-scale` imports resolve.
const dir = here('../node_modules/.chart-check/');

let failed = 0;
const check = (name, ok) => {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}`);
  if (!ok) failed++;
};
const count = (s, re) => (s.match(re) ?? []).length;

// the component's own source: only scaleLinear from d3-scale, no time scale, no date parsing, no red
const imports = [...src.matchAll(/import\s+([^;]+?)\s+from\s+'([^']+)'/g)].map((m) => [m[1], m[2]]);
const d3 = imports.filter(([, from]) => from.startsWith('d3'));
check("imports only { scaleLinear } from 'd3-scale'", d3.length === 1 && d3[0][1] === 'd3-scale' && d3[0][0].replace(/\s/g, '') === '{scaleLinear}');
check('no time scale, no Date.parse, no new Date, no destructive colour', !/scaleTime|Date\.parse|new Date\(|--heart/.test(src));

const day = (n) => 20000 + n;
const row = (i, raw, trend) => ({ date_text: `Day ${i}`, raw_text: `${raw} kg`, trend_text: `${trend} kg` });
const base = { xMin: day(0), xMax: day(10), yMin: 77, yMax: 81, goal: null, goalText: null, label: 'Weight, 3 months: trend from 79.5 to 79.0 kg.', unitText: 'kg', emptyText: null, table: [], asTable: false };
// three weigh-ins with a gap between the second and the third, and one point whose trend is missing
const gappy = [
  { day: day(0), raw: 80, trend: 79.5 },
  { day: day(2), raw: 79, trend: null },
  { day: day(9), raw: 79.2, trend: 79.0 },
  { day: day(10), raw: 79.1, trend: 78.9 },
];

try {
  mkdirSync(dir, { recursive: true });
  // the chart's date helper lives in api.ts, which needs the Tauri bridge; the check gives it a stand-in that prints the day number
  const stub = dir + 'api-stub.mjs';
  writeFileSync(stub, 'export const dayLabel = (d) => "D" + d;\n');
  const { js } = compile(src.replace(/from '\.\/api'/, `from '${pathToFileURL(stub).href}'`), { generate: 'server', filename: 'LineChart.svelte' });
  const file = dir + 'linechart.server.mjs';
  writeFileSync(file, js.code);
  const { default: LineChart } = await import(pathToFileURL(file).href);
  const draw = (props) => render(LineChart, { props: { ...base, ...props } }).body;

  const a = draw({ points: gappy, goal: 74, goalText: 'Goal 74 kg', yMin: 73.5 });
  check('role="img" carries the crate label', count(a, /role="img"/g) === 1 && a.includes('aria-label="Weight, 3 months: trend from 79.5 to 79.0 kg."'));
  check('four weigh-ins draw four dots', count(a, /<circle/g) === 4 && count(a, /r="3"/g) === 4);
  check('one trend path, joining only the points that have a trend', count(a, /<path/g) === 1 && count((a.match(/<path[^>]*\sd="([^"]*)"/) ?? [, ''])[1], /[ML]/g) === 3);
  check('one dashed goal line with its label', count(a, /stroke-dasharray="4 4"/g) === 1 && a.includes('Goal 74 kg'));
  check('gridlines and day-number date labels are drawn', count(a, /<line[^>]*grid-line/g) >= 2 && /D200\d\d/.test(a));
  check('no NaN, Infinity or undefined in the markup', !/NaN|Infinity|undefined/.test(a));
  check('no area, gradient or red', !/<linearGradient|<radialGradient|fill-opacity|#[fF]{2}0{4}|red/.test(a));
  check('a keyboard overlay with a polite status, and no tooltip until a point is picked', a.includes('role="application"') && count(a, /role="status"/g) === 1 && !a.includes('class="tip'));

  const noGoal = draw({ points: gappy });
  check('no goal line when no goal is set', count(noGoal, /stroke-dasharray/g) === 0);

  const one = draw({ points: [{ day: day(5), raw: 79, trend: 79 }], xMin: day(5), xMax: day(5) });
  check('one point: a dot and no trend path', count(one, /<circle/g) === 1 && count(one, /<path/g) === 0);
  check('one point: nothing is NaN even with a one-day domain', !/NaN|Infinity/.test(one));

  const none = draw({ points: [], emptyText: 'No weigh-ins in this range. Try a longer range.' });
  check('no points: the empty text and no chart', none.includes('No weigh-ins in this range. Try a longer range.') && !none.includes('<svg') && !none.includes('role="img"'));

  const tbl = draw({ points: gappy, asTable: true, table: [row(0, 80, 79.5), row(2, 79, 79.3), row(9, 79.2, 79.0), row(10, 79.1, 78.9)] });
  check('table mode: a table with a header and one row per point, no svg', count(tbl, /<table/g) === 1 && count(tbl, /<tbody>[\s\S]*<\/tbody>/g) === 1 && count(tbl, /<tr>/g) === 5 && !tbl.includes('<svg'));
  check('table mode: the same text the crate sent, and the label as a caption', tbl.includes('Day 9') && tbl.includes('79.2 kg') && tbl.includes('78.9 kg') && tbl.includes('<caption'));
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
console.log('chart-check: all passed');
