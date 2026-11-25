// Test page: the recipe_box suite against real OPFS (?wasm=<url>&args=<test args>), or the bridge's own checks (?bridge).
import { DATA_BYTES } from '../src/web/opfs-protocol';

const out = document.querySelector('pre')!;
const q = new URLSearchParams(location.search);
const wasmUrl = q.get('wasm') ?? '';
const args = (q.get('args') ?? '').split(' ').filter(Boolean);

async function run() {
  if (!crossOriginIsolated) throw new Error('not cross-origin isolated: SharedArrayBuffer is unavailable');
  // a clean /tmp for every run
  const root = await navigator.storage.getDirectory();
  await root.removeEntry('tiffin-test-tmp', { recursive: true }).catch(() => {});
  const ctrl = new SharedArrayBuffer(16);
  const data = new SharedArrayBuffer(DATA_BYTES);
  const channel = new MessageChannel();
  const io = new Worker(new URL('../src/web/opfs-io.worker.ts', import.meta.url), { type: 'module' });
  await new Promise<void>((ok) => {
    io.onmessage = (e) => e.data.type === 'ready' && ok();
    io.postMessage({ type: 'init', ctrl, data, port: channel.port1, mounts: { '/tmp': 'tiffin-test-tmp' } }, [channel.port1]);
  });
  if (q.has('bridge')) {
    const unit = new Worker(new URL('./bridge.worker.ts', import.meta.url), { type: 'module' });
    unit.onmessage = (e) => {
      out.textContent += (e.data.results as string[]).join('\n') + '\n';
      document.title = (e.data.results as string[]).some((r) => r.startsWith('FAIL')) ? 'done failed' : 'done 0';
    };
    unit.postMessage({ ctrl, data, port: channel.port2 }, [channel.port2]);
    return;
  }
  const runner = new Worker(new URL('./run.worker.ts', import.meta.url), { type: 'module' });
  runner.onmessage = (e) => {
    if (e.data.type === 'line') out.textContent += e.data.line + '\n';
    if (e.data.type === 'done') document.title = `done ${e.data.code}`;
  };
  runner.postMessage({ wasmUrl, args, ctrl, data, port: channel.port2 }, [channel.port2]);
}

run().catch((e) => {
  out.textContent += `failed: ${e}\n`;
  document.title = 'done failed';
});
