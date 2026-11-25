// Kill a saving library at random moments, the way closing the tab does (both workers stop at once),
// then load what is left. Every round must find a whole library holding every save that finished,
// with a whole .bak and whole snapshots. (A browser or power crash is not modelled: only a worker kill.)
// ?wasm=<crash_save.wasm url>&rounds=<n>
import { DATA_BYTES } from '../src/web/opfs-protocol';

const out = document.querySelector('pre')!;
const q = new URLSearchParams(location.search);
const wasmUrl = q.get('wasm') ?? '';
const rounds = Number(q.get('rounds') ?? '20');
const FOLDER = 'tiffin-crash';
const say = (s: string) => (out.textContent += s + '\n');

/** Start the two workers on /tmp; resolves with them and a promise for the program's output. */
async function start(args: string[], onLine?: (l: string) => void) {
  const ctrl = new SharedArrayBuffer(16);
  const data = new SharedArrayBuffer(DATA_BYTES);
  const channel = new MessageChannel();
  const io = new Worker(new URL('../src/web/opfs-io.worker.ts', import.meta.url), { type: 'module' });
  await new Promise<void>((ok) => {
    io.onmessage = (e) => e.data.type === 'ready' && ok();
    io.postMessage({ type: 'init', ctrl, data, port: channel.port1, mounts: { '/tmp': FOLDER } }, [channel.port1]);
  });
  const runner = new Worker(new URL('./run.worker.ts', import.meta.url), { type: 'module' });
  const lines: string[] = [];
  const done = new Promise<string[]>((ok) => {
    runner.onmessage = (e) => {
      if (e.data.type === 'line') {
        lines.push(e.data.line);
        onLine?.(e.data.line);
      }
      if (e.data.type === 'done') ok(lines);
    };
  });
  runner.postMessage({ wasmUrl, args, ctrl, data, port: channel.port2 }, [channel.port2]);
  return { kill: () => (runner.terminate(), io.terminate()), done };
}

async function run() {
  const root = await navigator.storage.getDirectory();
  await root.removeEntry(FOLDER, { recursive: true }).catch(() => {});
  let bad = 0;
  let had = 0; // recipes in the last version found
  for (let r = 1; r <= rounds; r++) {
    let saved = 0;
    const saver = await start(['save', '/tmp/lib'], (l) => l.startsWith('saved') && saved++);
    // somewhere inside a save: after at least one finished, at a random moment
    const t0 = performance.now();
    while (saved < 1 && performance.now() - t0 < 20_000) await new Promise((ok) => setTimeout(ok, 5));
    await new Promise((ok) => setTimeout(ok, Math.random() * 400));
    saver.kill();
    const checker = await start(['check', '/tmp/lib']);
    const found = (await checker.done).join(' ').trim();
    const n = Number(found.split(' ')[1] ?? -1);
    // whole, and no save that finished was lost: the kill may only take the one in progress
    const kept = found.startsWith('OK') && n >= had + saved;
    const broken = / bak-broken| snapshot-broken/.test(found);
    if (!kept || broken) bad++;
    say(`round ${r}: ${saved} saves before the kill, then found: ${found}${kept ? '' : `  LOST (expected at least ${had + saved})`}`);
    if (found.startsWith('OK')) had = n;
  }
  say(bad ? `${bad} rounds found a broken library` : `all ${rounds} rounds found a whole library`);
  document.title = bad ? 'done failed' : 'done 0';
}

run().catch((e) => {
  say(`failed: ${e}`);
  document.title = 'done failed';
});
