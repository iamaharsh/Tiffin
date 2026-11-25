// The message format between the WebAssembly worker and the OPFS worker, in one shared buffer:
// [json length u32][bin length u32][json][bin]. `ctrl[state]` is 1 while a request waits, 2 once answered.

export const HEADER = { state: 0 } as const;
/** Shared buffer for one request or reply. Reads and writes larger than CHUNK are split. */
export const DATA_BYTES = 9 * 1024 * 1024;
export const CHUNK = 8 * 1024 * 1024;
/** A request the OPFS worker hasn't answered in this long means it is gone; the bridge then refuses everything. */
export const REPLY_MS = 60_000;

type Where = { mount: string; parts: string[] };
export type IoRequest = Where &
  (
    | { op: 'stat' | 'mkdir' | 'rmdir' | 'unlink' | 'list' }
    | { op: 'open'; create: boolean; excl: boolean; trunc: boolean; dir: boolean; write: boolean }
    // `id` is the open file `open` answered with: a descriptor keeps meaning that file even after the path is reused
    | { op: 'flush' | 'close' | 'fstat'; id: number }
    | { op: 'read'; id: number; at: number; len: number }
    | { op: 'write'; id: number; at: number; append: boolean }
    | { op: 'truncate'; id: number; len: number }
    | { op: 'rename'; toMount: string; toParts: string[] }
  );

export type IoReply = {
  err?: 'NOENT' | 'EXIST' | 'NOTDIR' | 'ISDIR' | 'NOTEMPTY' | 'BADF' | 'BUSY' | 'NOSPC' | 'NOTSUP' | 'NOTCAPABLE' | 'IO';
  kind?: 'file' | 'dir';
  id?: number;
  size?: number;
  mtimeMs?: number;
  n?: number;
  names?: { name: string; dir: boolean }[];
};

const enc = new TextEncoder();
const dec = new TextDecoder();

export function encode(data: Uint8Array, msg: unknown, bin?: Uint8Array) {
  const json = enc.encode(JSON.stringify(msg));
  const binLen = bin?.byteLength ?? 0;
  if (8 + json.byteLength + binLen > data.byteLength) throw new Error('bridge message too large');
  const view = new DataView(data.buffer, data.byteOffset);
  view.setUint32(0, json.byteLength, true);
  view.setUint32(4, binLen, true);
  data.set(json, 8);
  if (bin) data.set(bin, 8 + json.byteLength);
}

export function decode<T>(data: Uint8Array): { req: T; bin: Uint8Array } {
  const view = new DataView(data.buffer, data.byteOffset);
  const jsonLen = view.getUint32(0, true);
  const binLen = view.getUint32(4, true);
  // copy out of shared memory: TextDecoder refuses shared views, and the buffer is reused next call
  const req = JSON.parse(dec.decode(data.slice(8, 8 + jsonLen))) as T;
  return { req, bin: data.slice(8 + jsonLen, 8 + jsonLen + binLen) };
}
