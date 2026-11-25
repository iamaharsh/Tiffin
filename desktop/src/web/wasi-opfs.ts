// WASI files and folders backed by the browser's private file system (OPFS), for recipe_box built
// for wasm32-wasip1. Rust's std::fs is synchronous, but OPFS folder calls are async, so each call
// goes through a sync bridge: this worker writes the request into shared memory and blocks on
// Atomics.wait while opfs-io.worker.ts does it. A save therefore returns only once OPFS has it.
// Needs cross-origin isolation (SharedArrayBuffer) and must run in a worker (Atomics.wait).

import { Fd, wasi, type WASI } from '@bjorn3/browser_wasi_shim';
import { CHUNK, HEADER, REPLY_MS, decode, encode, type IoReply, type IoRequest } from './opfs-protocol';

const ERRNO: Record<NonNullable<IoReply['err']>, number> = {
  NOENT: wasi.ERRNO_NOENT,
  EXIST: wasi.ERRNO_EXIST,
  NOTDIR: wasi.ERRNO_NOTDIR,
  ISDIR: wasi.ERRNO_ISDIR,
  NOTEMPTY: wasi.ERRNO_NOTEMPTY,
  BADF: wasi.ERRNO_BADF,
  BUSY: wasi.ERRNO_BUSY,
  NOSPC: wasi.ERRNO_NOSPC,
  NOTSUP: wasi.ERRNO_NOTSUP,
  NOTCAPABLE: wasi.ERRNO_NOTCAPABLE,
  IO: wasi.ERRNO_IO,
};

export class Bridge {
  private ctrl: Int32Array;
  private data: Uint8Array;
  /** Set once a reply never came: a late one could be mistaken for the next call's, so nothing more is sent. */
  private broken = false;
  constructor(ctrl: SharedArrayBuffer, data: SharedArrayBuffer, private port: MessagePort) {
    this.ctrl = new Int32Array(ctrl);
    this.data = new Uint8Array(data);
  }

  call(req: IoRequest, bin?: Uint8Array): { reply: IoReply; bin: Uint8Array } {
    if (this.broken) return { reply: { err: 'IO' }, bin: new Uint8Array() };
    encode(this.data, req, bin);
    Atomics.store(this.ctrl, HEADER.state, 1);
    this.port.postMessage(0);
    const deadline = Date.now() + REPLY_MS;
    while (Atomics.load(this.ctrl, HEADER.state) === 1) {
      const left = deadline - Date.now();
      if (left <= 0) {
        this.broken = true;
        return { reply: { err: 'IO' }, bin: new Uint8Array() };
      }
      Atomics.wait(this.ctrl, HEADER.state, 1, left);
    }
    const { req: reply, bin: out } = decode<IoReply>(this.data);
    Atomics.store(this.ctrl, HEADER.state, 0);
    return { reply, bin: out };
  }
}

const errno = (r: IoReply) => (r.err ? ERRNO[r.err] : wasi.ERRNO_SUCCESS);

/** A WASI path relative to a folder: no absolute paths, no climbing above the mount. */
function join(base: string[], path: string): { ret: number; parts: string[] } {
  if (path.startsWith('/')) return { ret: wasi.ERRNO_NOTCAPABLE, parts: [] };
  if (path.includes('\0')) return { ret: wasi.ERRNO_INVAL, parts: [] };
  const parts = [...base];
  for (const c of path.split('/')) {
    if (c === '' || c === '.') continue;
    if (c === '..') {
      if (parts.length <= 0) return { ret: wasi.ERRNO_NOTCAPABLE, parts: [] };
      parts.pop();
    } else parts.push(c);
  }
  return { ret: wasi.ERRNO_SUCCESS, parts };
}

function ino(mount: string, parts: string[]): bigint {
  let h = 0xcbf29ce484222325n; // FNV-1a over the full path: the same file keeps its number
  for (const b of new TextEncoder().encode(mount + '/' + parts.join('/'))) h = ((h ^ BigInt(b)) * 0x100000001b3n) & 0xffffffffffffffffn;
  return h;
}

/** A filestat with the real modification time. (The shim's own Filestat writes the time fields at the wrong offsets.) */
class Stat {
  constructor(readonly ino: bigint, readonly filetype: number, readonly size: bigint, readonly mtim: bigint) {}
  write_bytes(view: DataView, ptr: number) {
    view.setBigUint64(ptr, 0n, true); // dev
    view.setBigUint64(ptr + 8, this.ino, true);
    view.setUint8(ptr + 16, this.filetype);
    view.setBigUint64(ptr + 24, 1n, true); // nlink
    view.setBigUint64(ptr + 32, this.size, true);
    view.setBigUint64(ptr + 40, this.mtim, true); // atim
    view.setBigUint64(ptr + 48, this.mtim, true);
    view.setBigUint64(ptr + 56, this.mtim, true); // ctim
  }
}

function statOf(bridge: Bridge, mount: string, parts: string[]): { ret: number; filestat: Stat | null } {
  const { reply: r } = bridge.call({ op: 'stat', mount, parts });
  if (r.err) return { ret: errno(r), filestat: null };
  const type = r.kind === 'dir' ? wasi.FILETYPE_DIRECTORY : wasi.FILETYPE_REGULAR_FILE;
  return { ret: 0, filestat: new Stat(ino(mount, parts), type, BigInt(r.size ?? 0), BigInt(Math.round(r.mtimeMs ?? 0)) * 1_000_000n) };
}

export class OpfsDir extends Fd {
  private listing: { name: string; dir: boolean }[] = [];
  constructor(readonly bridge: Bridge, private mount: string, private parts: string[] = [], private preopen?: string) {
    super();
  }

  fd_prestat_get() {
    if (this.preopen === undefined) return super.fd_prestat_get();
    return { ret: 0, prestat: wasi.Prestat.dir(this.preopen) };
  }
  fd_fdstat_get() {
    return { ret: 0, fdstat: new wasi.Fdstat(wasi.FILETYPE_DIRECTORY, 0) };
  }
  fd_filestat_get() {
    return statOf(this.bridge, this.mount, this.parts) as unknown as ReturnType<Fd['fd_filestat_get']>;
  }
  fd_readdir_single(cookie: bigint) {
    if (cookie === 0n) {
      const { reply: r } = this.bridge.call({ op: 'list', mount: this.mount, parts: this.parts });
      if (r.err) return { ret: errno(r), dirent: null };
      this.listing = [{ name: '.', dir: true }, { name: '..', dir: true }, ...(r.names ?? [])];
    }
    const e = this.listing[Number(cookie)];
    if (!e) return { ret: 0, dirent: null };
    const type = e.dir ? wasi.FILETYPE_DIRECTORY : wasi.FILETYPE_REGULAR_FILE;
    return { ret: 0, dirent: new wasi.Dirent(cookie + 1n, ino(this.mount, [...this.parts, e.name]), e.name, type) };
  }
  path_filestat_get(_flags: number, path: string) {
    const { ret, parts } = join(this.parts, path);
    if (ret) return { ret, filestat: null };
    return statOf(this.bridge, this.mount, parts) as unknown as ReturnType<Fd['path_filestat_get']>;
  }
  path_open(_dirflags: number, path: string, oflags: number, rights: bigint, _inherit: bigint, fdflags: number) {
    const { ret, parts } = join(this.parts, path);
    if (ret) return { ret, fd_obj: null };
    if (parts.length === 0) return { ret: 0, fd_obj: new OpfsDir(this.bridge, this.mount, []) };
    const { reply: r } = this.bridge.call({
      op: 'open',
      mount: this.mount,
      parts,
      create: (oflags & wasi.OFLAGS_CREAT) !== 0,
      excl: (oflags & wasi.OFLAGS_EXCL) !== 0,
      trunc: (oflags & wasi.OFLAGS_TRUNC) !== 0,
      dir: (oflags & wasi.OFLAGS_DIRECTORY) !== 0,
      write: (rights & BigInt(wasi.RIGHTS_FD_WRITE)) !== 0n,
    });
    if (r.err) return { ret: errno(r), fd_obj: null };
    if (r.kind === 'dir') return { ret: 0, fd_obj: new OpfsDir(this.bridge, this.mount, parts) };
    return { ret: 0, fd_obj: new OpfsFile(this.bridge, this.mount, parts, r.id ?? 0, (fdflags & wasi.FDFLAGS_APPEND) !== 0) };
  }
  path_create_directory(path: string) {
    return this.simple('mkdir', path);
  }
  path_remove_directory(path: string) {
    return this.simple('rmdir', path);
  }
  path_unlink_file(path: string) {
    return this.simple('unlink', path);
  }
  path_filestat_set_times() {
    return wasi.ERRNO_SUCCESS; // shortcut: setting times is ignored; recipe_box never sets them
  }
  private simple(op: 'mkdir' | 'rmdir' | 'unlink', path: string) {
    const { ret, parts } = join(this.parts, path);
    if (ret) return ret;
    return errno(this.bridge.call({ op, mount: this.mount, parts }).reply);
  }

  /** For path_rename: where a path inside this folder lives. */
  locate(path: string) {
    const { ret, parts } = join(this.parts, path);
    return { ret, mount: this.mount, parts };
  }
}

export class OpfsFile extends Fd {
  private pos = 0n;
  private closed = false;
  constructor(private bridge: Bridge, private mount: string, private parts: string[], private id: number, private append: boolean) {
    super();
  }

  private size(): bigint {
    return this.stat().filestat?.size ?? 0n;
  }
  /** This open file's own size and time, even if its path has since been reused. */
  private stat(): { ret: number; filestat: Stat | null } {
    const { reply: r } = this.bridge.call({ op: 'fstat', mount: this.mount, parts: this.parts, id: this.id });
    if (r.err) return { ret: errno(r), filestat: null };
    return { ret: 0, filestat: new Stat(ino(this.mount, this.parts), wasi.FILETYPE_REGULAR_FILE, BigInt(r.size ?? 0), BigInt(Math.round(r.mtimeMs ?? 0)) * 1_000_000n) };
  }
  fd_fdstat_get() {
    return { ret: 0, fdstat: new wasi.Fdstat(wasi.FILETYPE_REGULAR_FILE, this.append ? wasi.FDFLAGS_APPEND : 0) };
  }
  fd_filestat_get() {
    return this.stat() as unknown as ReturnType<Fd['fd_filestat_get']>;
  }
  fd_pread(size: number, offset: bigint) {
    const { reply: r, bin } = this.bridge.call({ op: 'read', mount: this.mount, parts: this.parts, id: this.id, at: Number(offset), len: Math.min(size, CHUNK) });
    if (r.err) return { ret: errno(r), data: new Uint8Array() };
    return { ret: 0, data: bin };
  }
  fd_read(size: number) {
    const out = this.fd_pread(size, this.pos);
    this.pos += BigInt(out.data.byteLength);
    return out;
  }
  fd_pwrite(data: Uint8Array, offset: bigint) {
    return this.writeAll(data, offset, false);
  }
  fd_write(data: Uint8Array) {
    const out = this.writeAll(data, this.pos, this.append);
    if (out.ret === 0) this.pos = this.append ? this.size() : this.pos + BigInt(out.nwritten);
    return out;
  }
  private writeAll(data: Uint8Array, offset: bigint, append: boolean) {
    let done = 0;
    while (done < data.byteLength || data.byteLength === 0) {
      const part = data.subarray(done, done + CHUNK);
      const { reply: r } = this.bridge.call({ op: 'write', mount: this.mount, parts: this.parts, id: this.id, at: Number(offset) + done, append }, part);
      if (r.err) return { ret: errno(r), nwritten: done };
      done += r.n ?? 0;
      if (data.byteLength === 0 || (r.n ?? 0) === 0) break;
    }
    return { ret: 0, nwritten: done };
  }
  fd_seek(offset: bigint, whence: number) {
    const base = whence === wasi.WHENCE_SET ? 0n : whence === wasi.WHENCE_CUR ? this.pos : whence === wasi.WHENCE_END ? this.size() : -1n;
    if (base < 0n || base + offset < 0n) return { ret: wasi.ERRNO_INVAL, offset: 0n };
    this.pos = base + offset;
    return { ret: 0, offset: this.pos };
  }
  fd_tell() {
    return { ret: 0, offset: this.pos };
  }
  fd_filestat_set_size(size: bigint) {
    return errno(this.bridge.call({ op: 'truncate', mount: this.mount, parts: this.parts, id: this.id, len: Number(size) }).reply);
  }
  fd_allocate(offset: bigint, len: bigint) {
    return this.size() < offset + len ? this.fd_filestat_set_size(offset + len) : wasi.ERRNO_SUCCESS;
  }
  fd_sync() {
    return errno(this.bridge.call({ op: 'flush', mount: this.mount, parts: this.parts, id: this.id }).reply);
  }
  fd_close() {
    if (this.closed) return wasi.ERRNO_SUCCESS;
    this.closed = true;
    return errno(this.bridge.call({ op: 'close', mount: this.mount, parts: this.parts, id: this.id }).reply);
  }
}

/** The shim renames by unlinking then linking; here a rename is one OPFS move, so it is atomic. */
export function useAtomicRename(w: WASI) {
  const imports = w.wasiImport as Record<string, unknown>;
  imports.path_rename = (fd: number, oldPtr: number, oldLen: number, newFd: number, newPtr: number, newLen: number) => {
    const from = w.fds[fd];
    const to = w.fds[newFd];
    if (!(from instanceof OpfsDir) || !(to instanceof OpfsDir)) return wasi.ERRNO_BADF;
    const mem = new Uint8Array((w.inst.exports.memory as WebAssembly.Memory).buffer);
    const text = (p: number, n: number) => new TextDecoder().decode(mem.slice(p, p + n));
    const a = from.locate(text(oldPtr, oldLen));
    const b = to.locate(text(newPtr, newLen));
    if (a.ret) return a.ret;
    if (b.ret) return b.ret;
    return errno(from.bridge.call({ op: 'rename', mount: a.mount, parts: a.parts, toMount: b.mount, toParts: b.parts }).reply);
  };
}
