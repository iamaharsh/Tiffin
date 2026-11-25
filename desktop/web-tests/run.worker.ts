// Runs one wasm32-wasip1 program (here: recipe_box's test suite) with /tmp on OPFS, and reports its output.
import { ConsoleStdout, File, OpenFile, WASI, WASIProcExit } from '@bjorn3/browser_wasi_shim';
import { Bridge, OpfsDir, useAtomicRename } from '../src/web/wasi-opfs';

self.onmessage = async (ev: MessageEvent) => {
  const { wasmUrl, args, ctrl, data, port } = ev.data;
  const say = (line: string) => (self as unknown as Worker).postMessage({ type: 'line', line });
  const bridge = new Bridge(ctrl, data, port);
  const fds = [new OpenFile(new File([])), ConsoleStdout.lineBuffered(say), ConsoleStdout.lineBuffered(say), new OpfsDir(bridge, '/tmp', [], '/tmp')];
  const w = new WASI(['recipe_box.wasm', ...args], [], fds);
  useAtomicRename(w);
  const { instance } = await WebAssembly.instantiateStreaming(fetch(wasmUrl), { wasi_snapshot_preview1: w.wasiImport });
  let code = 0;
  try {
    code = w.start(instance as unknown as { exports: { memory: WebAssembly.Memory; _start: () => unknown } });
  } catch (e) {
    if (e instanceof WASIProcExit) code = e.code;
    else {
      say(`crashed: ${e}`);
      code = 101;
    }
  }
  (self as unknown as Worker).postMessage({ type: 'done', code });
};
