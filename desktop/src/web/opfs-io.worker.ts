// The OPFS side of the sync bridge (see wasi-opfs.ts). Every OPFS call that touches a folder is async,
// so the WebAssembly worker can't make them itself: it writes a request into shared memory, pings this
// worker, and waits. This worker does the call and wakes it. One request at a time, in order.

import { HEADER, decode, encode, type IoRequest, type IoReply } from './opfs-protocol';

// shortcut: the DOM typings this project builds with lack the worker-only sync handle; these are the members used.
type SyncHandle = {
  read(buf: Uint8Array, opts: { at: number }): number;
  write(buf: Uint8Array, opts: { at: number }): number;
  truncate(size: number): void;
  getSize(): number;
  flush(): void;
  close(): void;
};
type FileHandle = FileSystemFileHandle & { createSyncAccessHandle(): Promise<SyncHandle>; move(dir: FileSystemDirectoryHandle, name: string): Promise<void> };

/** An open file. OPFS allows one sync handle per file, so descriptors opened on the same path share it. */
type Open = { id: number; key: string; handle: SyncHandle; users: number; mtimeMs: number; gone: boolean };
/** Open files by the id `open` handed out, and the one currently at each path. */
const byId = new Map<number, Open>();
const atPath = new Map<string, Open>();
let nextId = 1;

/** The open file a request names; a file since moved away or removed answers BADF, never whatever is at its old path now. */
function live(id: number): Open {
  const o = byId.get(id);
  if (!o || o.gone) throw new Fail('BADF');
  return o;
}
let mounts: Record<string, FileSystemDirectoryHandle> = {};

class Fail extends Error {
  constructor(readonly code: IoReply['err']) {
    super(code);
  }
}

function why(e: unknown): IoReply['err'] {
  if (e instanceof Fail) return e.code;
  const name = (e as DOMException)?.name;
  if (name === 'NotFoundError') return 'NOENT';
  if (name === 'TypeMismatchError') return 'NOTDIR';
  if (name === 'InvalidModificationError') return 'NOTEMPTY';
  if (name === 'NoModificationAllowedError') return 'BUSY';
  if (name === 'QuotaExceededError') return 'NOSPC';
  return 'IO';
}

/** The folder holding `parts` (all but the last), and the last part's name. */
async function parentOf(mount: string, parts: string[], create = false) {
  let dir = mounts[mount];
  if (!dir) throw new Fail('NOTCAPABLE');
  for (const p of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(p, { create });
  return { dir, name: parts[parts.length - 1] };
}

async function dirAt(mount: string, parts: string[]) {
  if (parts.length === 0) {
    const root = mounts[mount];
    if (!root) throw new Fail('NOTCAPABLE');
    return root;
  }
  const { dir, name } = await parentOf(mount, parts);
  return dir.getDirectoryHandle(name);
}

/** A file or a folder at `parts`, or NOENT. */
async function entryAt(mount: string, parts: string[]): Promise<FileSystemHandle> {
  if (parts.length === 0) return dirAt(mount, parts);
  const { dir, name } = await parentOf(mount, parts);
  try {
    return await dir.getFileHandle(name);
  } catch (e) {
    if ((e as DOMException)?.name !== 'TypeMismatchError') throw e;
    return dir.getDirectoryHandle(name);
  }
}

const key = (mount: string, parts: string[]) => mount + '/' + parts.join('/');

async function handle(req: IoRequest, bin: Uint8Array): Promise<{ reply: IoReply; bin?: Uint8Array }> {
  const k = key(req.mount, req.parts);
  switch (req.op) {
    case 'stat': {
      const o = atPath.get(k);
      if (o) return { reply: { kind: 'file', size: o.handle.getSize(), mtimeMs: o.mtimeMs } };
      const e = await entryAt(req.mount, req.parts);
      if (e.kind === 'directory') return { reply: { kind: 'dir', size: 0, mtimeMs: 0 } };
      const f = await (e as FileSystemFileHandle).getFile();
      return { reply: { kind: 'file', size: f.size, mtimeMs: f.lastModified } };
    }
    case 'open': {
      const { dir, name } = await parentOf(req.mount, req.parts);
      if (req.dir) {
        await dir.getDirectoryHandle(name, { create: req.create });
        return { reply: { kind: 'dir' } };
      }
      const existing = atPath.get(k);
      let exists = !!existing;
      if (!exists) {
        try {
          await dir.getFileHandle(name);
          exists = true;
        } catch (e) {
          const n = (e as DOMException)?.name;
          if (n === 'TypeMismatchError') {
            // a folder: opening it as a folder is fine, as a file it isn't
            if (req.create && req.excl) throw new Fail('EXIST');
            if (req.write) throw new Fail('ISDIR');
            return { reply: { kind: 'dir' } };
          }
          if (n !== 'NotFoundError') throw e;
        }
      }
      if (exists && req.create && req.excl) throw new Fail('EXIST');
      if (!exists && !req.create) throw new Fail('NOENT');
      let o = existing;
      if (!o) {
        const fh = (await dir.getFileHandle(name, { create: true })) as FileHandle;
        const f = await fh.getFile();
        o = { id: nextId++, key: k, handle: await fh.createSyncAccessHandle(), users: 0, mtimeMs: f.lastModified, gone: false };
        atPath.set(k, o);
        byId.set(o.id, o);
      }
      o.users++;
      if (req.trunc) {
        o.handle.truncate(0);
        o.mtimeMs = Date.now();
      }
      return { reply: { kind: 'file', id: o.id, size: o.handle.getSize() } };
    }
    case 'read': {
      const o = live(req.id);
      const buf = new Uint8Array(req.len);
      const n = o.handle.read(buf, { at: req.at });
      return { reply: { n }, bin: buf.subarray(0, n) };
    }
    case 'write': {
      const o = live(req.id);
      const at = req.append ? o.handle.getSize() : req.at;
      const n = o.handle.write(bin, { at });
      o.mtimeMs = Date.now();
      return { reply: { n, size: o.handle.getSize() } };
    }
    case 'truncate': {
      const o = live(req.id);
      o.handle.truncate(req.len);
      o.mtimeMs = Date.now();
      return { reply: {} };
    }
    case 'fstat': {
      const o = live(req.id);
      return { reply: { kind: 'file', size: o.handle.getSize(), mtimeMs: o.mtimeMs } };
    }
    case 'flush': {
      live(req.id).handle.flush();
      return { reply: {} };
    }
    case 'close': {
      const o = byId.get(req.id);
      if (o && --o.users <= 0) {
        if (!o.gone) shut(o);
        byId.delete(o.id);
      }
      return { reply: {} };
    }
    case 'mkdir': {
      const { dir, name } = await parentOf(req.mount, req.parts);
      if (await exists(dir, name)) throw new Fail('EXIST');
      await dir.getDirectoryHandle(name, { create: true });
      return { reply: {} };
    }
    case 'rmdir': {
      const { dir, name } = await parentOf(req.mount, req.parts);
      const d = await dir.getDirectoryHandle(name);
      for await (const _ of (d as unknown as { keys(): AsyncIterable<string> }).keys()) throw new Fail('NOTEMPTY');
      await dir.removeEntry(name);
      return { reply: {} };
    }
    case 'unlink': {
      const { dir, name } = await parentOf(req.mount, req.parts);
      await dir.getFileHandle(name); // a folder is ISDIR (TypeMismatchError), not removed
      closeAll(k);
      await dir.removeEntry(name);
      return { reply: {} };
    }
    case 'list': {
      const d = await dirAt(req.mount, req.parts);
      const names: { name: string; dir: boolean }[] = [];
      for await (const [name, h] of (d as unknown as { entries(): AsyncIterable<[string, FileSystemHandle]> }).entries()) names.push({ name, dir: h.kind === 'directory' });
      names.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
      return { reply: { names } };
    }
    case 'rename': {
      // One OPFS move: the new name holds either the whole old file or the old target, never a mix.
      const from = await entryAt(req.mount, req.parts);
      const to = await parentOf(req.toMount, req.toParts);
      const toKey = key(req.toMount, req.toParts);
      if (from.kind === 'directory') throw new Fail('NOTSUP'); // the app never renames folders
      closeAll(k);
      closeAll(toKey);
      await (from as FileHandle).move(to.dir, to.name);
      return { reply: {} };
    }
  }
}

async function exists(dir: FileSystemDirectoryHandle, name: string) {
  try {
    await dir.getFileHandle(name);
    return true;
  } catch (e) {
    return (e as DOMException)?.name === 'TypeMismatchError';
  }
}

/** A file about to be moved or removed can't stay open (OPFS would refuse). Its descriptors then get BADF. */
function closeAll(k: string) {
  const o = atPath.get(k);
  if (o) shut(o);
}

function shut(o: Open) {
  o.handle.flush();
  o.handle.close();
  o.gone = true;
  if (atPath.get(o.key) === o) atPath.delete(o.key);
}

let ctrl: Int32Array;
let data: Uint8Array;

self.onmessage = async (ev: MessageEvent) => {
  const m = ev.data;
  if (m.type === 'init') {
    ctrl = new Int32Array(m.ctrl);
    data = new Uint8Array(m.data);
    mounts = {};
    const root = await navigator.storage.getDirectory();
    for (const [name, folder] of Object.entries(m.mounts as Record<string, string>)) mounts[name] = await root.getDirectoryHandle(folder, { create: true });
    (m.port as MessagePort).onmessage = () => serve();
    (self as unknown as Worker).postMessage({ type: 'ready' });
  }
};

async function serve() {
  try {
    const { req, bin } = decode<IoRequest>(data);
    const out = await handle(req, bin);
    encode(data, out.reply, out.bin);
  } catch (e) {
    // a reply that can't be sent is an error, never silence: the other side is blocked waiting
    encode(data, { err: why(e) });
  }
  Atomics.store(ctrl, HEADER.state, 2);
  Atomics.notify(ctrl, HEADER.state);
}
