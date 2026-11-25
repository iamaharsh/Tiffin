// WebAssembly worker for Tiffin on the web: runs the tiffin-web-wasm WASI reactor on top of the
// OPFS sync bridge (/data -> OPFS folder "tiffin-data") and answers command messages from transport.ts.

import { ConsoleStdout, File, OpenFile, WASI } from '@bjorn3/browser_wasi_shim';
import { Bridge, OpfsDir, useAtomicRename } from './wasi-opfs';

type ReactorExports = {
  memory: WebAssembly.Memory;
  alloc(len: number): number;
  dealloc(ptr: number, len: number): void;
  init(dirPtr: number, dirLen: number): number;
  call(namePtr: number, nameLen: number, argsPtr: number, argsLen: number): number;
  call_bytes(namePtr: number, nameLen: number, argPtr: number, argLen: number, binPtr: number, binLen: number): number;
  reply_ptr(): number;
  reply_len(): number;
  reply_is_err(): number;
};

type WorkerMsg =
  | { type: 'init'; wasmUrl: string; ctrl: SharedArrayBuffer; data: SharedArrayBuffer; port: MessagePort }
  | { type: 'call'; id: number; cmd: string; args: unknown }
  | { type: 'call-bytes'; id: number; cmd: string; arg: string; bin: Uint8Array; rawReply?: boolean };

const enc = new TextEncoder();
const dec = new TextDecoder();
const post = (msg: unknown, transfer?: Transferable[]) =>
  (self as unknown as Worker).postMessage(msg, transfer ?? []);

let ex: ReactorExports | null = null;

function put(r: ReactorExports, bytes: Uint8Array): [number, number] {
  const len = bytes.byteLength;
  if (len === 0) return [0, 0];
  const ptr = r.alloc(len) >>> 0;
  if (ptr === 0) throw new Error('Out of WebAssembly memory.');
  new Uint8Array(r.memory.buffer, ptr, len).set(bytes);
  return [ptr, len];
}

function free(r: ReactorExports, ptr: number, len: number) {
  if (len <= 0 || ptr === 0) return;
  try {
    r.dealloc(ptr, len);
  } catch {
    // if WASM already trapped, dealloc can trap too; ex is poisoned by the caller
  }
}

function takeReply(r: ReactorExports, rawLen: number): { bytes: Uint8Array; isErr: boolean } {
  const len = rawLen >>> 0;
  const ptr = len === 0 ? 0 : r.reply_ptr() >>> 0;
  const isErr = r.reply_is_err() !== 0;
  const bytes = len === 0 ? new Uint8Array() : new Uint8Array(r.memory.buffer, ptr, len).slice();
  return { bytes, isErr };
}

function errMsg(e: unknown): string {
  return String(e instanceof Error ? e.message : e);
}

async function instantiate(wasmUrl: string, imports: WebAssembly.Imports): Promise<WebAssembly.Instance> {
  const res = await fetch(wasmUrl);
  if (!res.ok) throw new Error(`Couldn't load Tiffin WebAssembly (${res.status}).`);
  if (typeof WebAssembly.instantiateStreaming === 'function' && res.headers.get('content-type')?.includes('application/wasm')) {
    const out = await WebAssembly.instantiateStreaming(res, imports);
    return out.instance;
  }
  const out = await WebAssembly.instantiate(await res.arrayBuffer(), imports);
  return out.instance;
}

self.onmessage = async (ev: MessageEvent<WorkerMsg>) => {
  const m = ev.data;
  if (m.type === 'init') {
    try {
      const bridge = new Bridge(m.ctrl, m.data, m.port);
      const fds = [
        new OpenFile(new File([])),
        ConsoleStdout.lineBuffered((line) => console.log('[tiffin-wasm]', line)),
        ConsoleStdout.lineBuffered((line) => console.error('[tiffin-wasm]', line)),
        new OpfsDir(bridge, '/data', [], '/data'),
      ];
      const w = new WASI(['tiffin.wasm'], [], fds, { debug: false });
      useAtomicRename(w);
      const instance = await instantiate(m.wasmUrl, { wasi_snapshot_preview1: w.wasiImport });
      w.initialize(instance as unknown as { exports: { memory: WebAssembly.Memory; _initialize?: () => unknown } });
      const exports = instance.exports as unknown as ReactorExports;
      const [dp, dl] = put(exports, enc.encode('/data'));
      let rc = 1;
      try {
        rc = exports.init(dp, dl);
      } finally {
        free(exports, dp, dl);
      }
      if (rc !== 0) {
        const { bytes } = takeReply(exports, exports.reply_len());
        throw new Error(dec.decode(bytes) || 'Tiffin could not open its browser storage.');
      }
      ex = exports;
      post({ type: 'ready' });
    } catch (e) {
      post({ type: 'init-error', error: errMsg(e) });
    }
    return;
  }

  const r = ex;
  if (!r) {
    post({ type: 'reply', id: m.id, ok: false, error: "Tiffin isn't ready yet." });
    return;
  }

  if (m.type === 'call') {
    let np = 0, nl = 0, ap = 0, al = 0;
    try {
      [np, nl] = put(r, enc.encode(m.cmd));
      [ap, al] = put(r, m.args == null ? new Uint8Array() : enc.encode(JSON.stringify(m.args)));
      const rlen = r.call(np, nl, ap, al);
      const { bytes, isErr } = takeReply(r, rlen);
      if (isErr) {
        post({ type: 'reply', id: m.id, ok: false, error: dec.decode(bytes) });
      } else {
        const value = bytes.byteLength === 0 ? null : JSON.parse(dec.decode(bytes));
        post({ type: 'reply', id: m.id, ok: true, value });
      }
    } catch (e) {
      if (e instanceof WebAssembly.RuntimeError) ex = null;
      post({ type: 'reply', id: m.id, ok: false, error: errMsg(e) });
    } finally {
      free(r, ap, al);
      free(r, np, nl);
    }
    return;
  }

  if (m.type === 'call-bytes') {
    let np = 0, nl = 0, ap = 0, al = 0, bp = 0, bl = 0;
    try {
      [np, nl] = put(r, enc.encode(m.cmd));
      [ap, al] = put(r, enc.encode(m.arg));
      [bp, bl] = put(r, m.bin);
      const rlen = r.call_bytes(np, nl, ap, al, bp, bl);
      const { bytes, isErr } = takeReply(r, rlen);
      if (isErr) {
        post({ type: 'reply', id: m.id, ok: false, error: dec.decode(bytes) });
      } else if (m.rawReply) {
        post({ type: 'reply', id: m.id, ok: true, value: bytes }, [bytes.buffer]);
      } else {
        post({ type: 'reply', id: m.id, ok: true, value: dec.decode(bytes) });
      }
    } catch (e) {
      if (e instanceof WebAssembly.RuntimeError) ex = null;
      post({ type: 'reply', id: m.id, ok: false, error: errMsg(e) });
    } finally {
      free(r, bp, bl);
      free(r, ap, al);
      free(r, np, nl);
    }
  }
};
